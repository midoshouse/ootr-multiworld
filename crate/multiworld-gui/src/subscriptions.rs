use {
    std::{
        any::TypeId,
        hash::{
            Hash as _,
            Hasher,
        },
        net::Ipv4Addr,
        sync::Arc,
        time::Duration,
    },
    async_proto::Protocol,
    futures::{
        future::{
            self,
            TryFutureExt,
        },
        stream::{
            self,
            BoxStream,
            StreamExt as _,
            TryStreamExt as _,
        },
    },
    iced_futures::subscription::Recipe,
    tokio::{
        net::{
            TcpListener,
            TcpStream,
        },
        pin,
        select,
        sync::Mutex,
        time::{
            Instant,
            interval_at,
            timeout,
        },
    },
    multiworld::frontend,
    crate::{
        Error,
        FrontendFlags,
        LoggingReader,
        Message,
    },
};

pub(crate) struct Connection {
    pub(crate) frontend: FrontendFlags,
    pub(crate) log: bool,
    pub(crate) connection_id: u8,
}

impl<H: Hasher, I> Recipe<H, I> for Connection {
    type Output = Message;

    fn hash(&self, state: &mut H) {
        TypeId::of::<Self>().hash(state);
        self.connection_id.hash(state);
    }

    fn stream(self: Box<Self>, _: BoxStream<'_, I>) -> BoxStream<'_, Message> {
        let frontend = self.frontend.clone();
        let log = self.log;
        stream::once(TcpStream::connect((Ipv4Addr::LOCALHOST, frontend::PORT)).map_err(Error::from))
            .and_then(move |mut tcp_stream| {
                let frontend = frontend.clone();
                async move {
                    frontend::PROTOCOL_VERSION.write(&mut tcp_stream).await?;
                    let client_version = u8::read(&mut tcp_stream).await?;
                    if client_version != frontend::PROTOCOL_VERSION {
                        return Err(Error::VersionMismatch { version: client_version, frontend })
                    }
                    let (reader, writer) = tcp_stream.into_split();
                    let reader = LoggingReader { context: "from frontend", inner: reader, log };
                    Ok(
                        stream::once(future::ok(Message::FrontendConnected(Arc::new(Mutex::new(writer)))))
                            .chain(stream::try_unfold(reader, |mut reader| async move {
                                Ok(Some((Message::Plugin(Box::new(reader.read::<frontend::ClientMessage>().await?)), reader)))
                            }))
                    )
                }
            })
            .try_flatten()
            .map(|res| match res {
                Ok(msg) => msg,
                Err(e) => Message::FrontendSubscriptionError(Arc::new(e)),
            })
            .chain(stream::pending())
            .boxed()
    }
}

pub(crate) struct Listener {
    pub(crate) frontend: FrontendFlags,
    pub(crate) log: bool,
    pub(crate) connection_id: u8,
}

impl<H: Hasher, I> Recipe<H, I> for Listener {
    type Output = Message;

    fn hash(&self, state: &mut H) {
        TypeId::of::<Self>().hash(state);
        self.connection_id.hash(state);
    }

    fn stream(self: Box<Self>, _: BoxStream<'_, I>) -> BoxStream<'_, Message> {
        let frontend = self.frontend.clone();
        let log = self.log;
        stream::once(TcpListener::bind((Ipv4Addr::LOCALHOST, frontend::PORT)))
            .and_then(move |listener| {
                let frontend = frontend.clone();
                future::ok(stream::try_unfold(listener, move |listener| {
                    let frontend = frontend.clone();
                    async move {
                        let (mut tcp_stream, _) = listener.accept().await?;
                        frontend::PROTOCOL_VERSION.write(&mut tcp_stream).await?;
                        let client_version = u8::read(&mut tcp_stream).await?;
                        if client_version != frontend::PROTOCOL_VERSION {
                            return Err(Error::VersionMismatch { version: client_version, frontend })
                        }
                        let (reader, writer) = tcp_stream.into_split();
                        let reader = LoggingReader { context: "from frontend", inner: reader, log };
                        Ok(Some((
                            stream::once(future::ok(Message::FrontendConnected(Arc::new(Mutex::new(writer)))))
                            .chain(stream::try_unfold(reader, |mut reader| async move {
                                Ok(Some((Message::Plugin(Box::new(reader.read::<frontend::ClientMessage>().await?)), reader)))
                            })),
                            listener,
                        )))
                    }
                }))
            })
            .try_flatten()
            .try_flatten()
            .map(|res| match res {
                Ok(msg) => msg,
                Err(e) => Message::FrontendSubscriptionError(Arc::new(e)),
            })
            .chain(stream::pending())
            .boxed()
    }
}

pub(crate) struct Client {
    pub(crate) log: bool,
    pub(crate) port: u16,
}

impl<H: Hasher, I> Recipe<H, I> for Client {
    type Output = Message;

    fn hash(&self, state: &mut H) {
        TypeId::of::<Self>().hash(state);
    }

    fn stream(self: Box<Self>, _: BoxStream<'_, I>) -> BoxStream<'_, Message> {
        let log = self.log;
        stream::once(TcpStream::connect((multiworld::ADDRESS_V4, self.port)))
            .err_into::<Error>()
            .and_then(move |mut tcp_stream| async move {
                multiworld::handshake(&mut tcp_stream).await?;
                let (reader, writer) = tcp_stream.into_split();
                let reader = LoggingReader { context: "from server", inner: reader, log };
                let writer = Arc::new(Mutex::new(writer));
                let interval = interval_at(Instant::now() + Duration::from_secs(30), Duration::from_secs(30));
                Ok(
                    stream::once(future::ok(Message::ServerConnected(writer.clone())))
                    .chain(stream::try_unfold((reader, writer, interval), |(reader, writer, mut interval)| async move {
                        pin! {
                            let read = timeout(Duration::from_secs(60), reader.read_owned::<multiworld::ServerMessage>());
                        }
                        Ok(loop {
                            select! {
                                res = &mut read => {
                                    let (reader, msg) = res??;
                                    break Some((Message::Server(msg), (reader, writer, interval)))
                                },
                                _ = interval.tick() => multiworld::ClientMessage::Ping.write(&mut *writer.lock().await).await?,
                            }
                        })
                    }))
                )
            })
            .try_flatten()
            .map(|res| match res {
                Ok(msg) => msg,
                Err(e) => Message::ServerSubscriptionError(Arc::new(e)),
            })
            .chain(stream::pending())
            .boxed()
    }
}
