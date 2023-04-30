use {
    std::{
        any::TypeId,
        hash::{
            Hash as _,
            Hasher,
        },
        net::Ipv4Addr,
        pin::pin,
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
            SplitSink,
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
        select,
        sync::Mutex,
        time::{
            Instant,
            interval_at,
            timeout,
        },
    },
    tokio_tungstenite::tungstenite,
    multiworld::{
        frontend,
        websocket_url,
    },
    crate::{
        Error,
        FrontendFlags,
        LoggingReader,
        LoggingStream,
        Message,
    },
};

pub(crate) type WsSink = SplitSink<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>, tungstenite::Message>;

pub(crate) struct Connection {
    pub(crate) port: u16,
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
        stream::once(TcpStream::connect((Ipv4Addr::LOCALHOST, self.port)).map_err(Error::from))
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
}

impl<H: Hasher, I> Recipe<H, I> for Client {
    type Output = Message;

    fn hash(&self, state: &mut H) {
        TypeId::of::<Self>().hash(state);
    }

    fn stream(self: Box<Self>, _: BoxStream<'_, I>) -> BoxStream<'_, Message> {
        let log = self.log;
        stream::once(tokio_tungstenite::connect_async(websocket_url()))
            .err_into::<Error>()
            .and_then(move |(websocket, _)| async move {
                let (sink, stream) = websocket.split();
                let stream = LoggingStream { context: "from server", inner: stream, log };
                let sink = Arc::new(Mutex::new(sink));
                let interval = interval_at(Instant::now() + Duration::from_secs(30), Duration::from_secs(30));
                Ok(
                    stream::once(future::ok(Message::ServerConnected(sink.clone())))
                    .chain(stream::try_unfold((stream, sink, interval), |(stream, sink, mut interval)| async move {
                        let mut read = pin!(timeout(Duration::from_secs(60), stream.read_owned()));
                        Ok(loop {
                            select! {
                                res = &mut read => {
                                    let (stream, msg) = res??;
                                    break Some((Message::Server(msg), (stream, sink, interval)))
                                },
                                _ = interval.tick() => multiworld::ClientMessage::Ping.write_ws(&mut *sink.lock().await).await?,
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
