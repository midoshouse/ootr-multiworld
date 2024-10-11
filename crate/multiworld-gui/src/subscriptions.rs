use {
    std::{
        any::TypeId,
        hash::Hash as _,
        io::prelude::*,
        net::Ipv4Addr,
        pin::{
            Pin,
            pin,
        },
        sync::Arc,
        time::Duration,
    },
    async_proto::Protocol,
    chrono::prelude::*,
    futures::{
        future::{
            self,
            TryFutureExt,
        },
        stream::{
            self,
            SplitSink,
            Stream,
            StreamExt as _,
            TryStreamExt as _,
        },
    },
    iced::advanced::subscription::{
        self,
        EventStream,
        Recipe,
    },
    log_lock::{
        Mutex,
        lock,
    },
    tokio::{
        net::{
            TcpListener,
            TcpStream,
        },
        select,
        time::{
            Instant,
            interval_at,
            timeout,
        },
    },
    tokio_tungstenite::tungstenite,
    url::Url,
    multiworld::{
        frontend::{
            self,
            Kind as Frontend,
        },
        user_agent,
        ws::latest as ws,
    },
    crate::{
        Error,
        FrontendWriter,
        LoggingReader,
        LoggingStream,
        Message,
    },
};

pub(crate) struct LoggingSubscription<T> {
    pub(crate) log: bool,
    pub(crate) context: &'static str,
    pub(crate) inner: T,
}

impl<T: Recipe<Output = Message> + 'static> Recipe for LoggingSubscription<T> {
    type Output = Message;

    fn hash(&self, state: &mut subscription::Hasher) {
        TypeId::of::<Self>().hash(state);
        self.inner.hash(state);
    }

    fn stream(self: Box<Self>, input: EventStream) -> Pin<Box<dyn Stream<Item = Message> + Send>> {
        let Self { log, context, inner } = *self;
        Box::new(inner).stream(input).then(move |msg| async move {
            if log {
                let _ = lock!(log = crate::LOG; writeln!(&*log, "{} subscription message {context}: {msg:?}", Utc::now().format("%Y-%m-%d %H:%M:%S")));
            }
            msg
        }).boxed()
    }
}

pub(crate) type WsSink = SplitSink<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>, tungstenite::Message>;

pub(crate) struct Connection {
    pub(crate) port: u16,
    pub(crate) frontend: Frontend,
    pub(crate) log: bool,
    pub(crate) connection_id: u8,
}

impl Recipe for Connection {
    type Output = Message;

    fn hash(&self, state: &mut subscription::Hasher) {
        TypeId::of::<Self>().hash(state);
        self.connection_id.hash(state);
    }

    fn stream(self: Box<Self>, _: EventStream) -> Pin<Box<dyn Stream<Item = Message> + Send>> {
        let frontend = self.frontend;
        let log = self.log;
        stream::once(TcpStream::connect((Ipv4Addr::LOCALHOST, self.port)).map_err(Error::from))
            .and_then(move |mut tcp_stream| async move {
                frontend::PROTOCOL_VERSION.write(&mut tcp_stream).await?;
                let client_version = u8::read(&mut tcp_stream).await?;
                if client_version != frontend::PROTOCOL_VERSION {
                    return Err(Error::VersionMismatch { version: client_version, frontend })
                }
                let (reader, writer) = tcp_stream.into_split();
                let reader = LoggingReader { context: "from frontend", inner: reader, log };
                Ok(
                    stream::once(future::ok(Message::FrontendConnected(FrontendWriter::Tcp(Arc::new(Mutex::new(writer))))))
                        .chain(stream::try_unfold(reader, |mut reader| async move {
                            Ok(Some((Message::Plugin(Box::new(reader.read::<frontend::ClientMessage>().await?)), reader)))
                        }))
                )
            })
            .try_flatten()
            .map(|res| res.unwrap_or_else(|e| Message::FrontendSubscriptionError(Arc::new(e))))
            .chain(stream::pending())
            .boxed()
    }
}

pub(crate) struct Listener {
    pub(crate) frontend: Frontend,
    pub(crate) log: bool,
    pub(crate) connection_id: u8,
}

impl Recipe for Listener {
    type Output = Message;

    fn hash(&self, state: &mut subscription::Hasher) {
        TypeId::of::<Self>().hash(state);
        self.connection_id.hash(state);
    }

    fn stream(self: Box<Self>, _: EventStream) -> Pin<Box<dyn Stream<Item = Message> + Send>> {
        let frontend = self.frontend;
        let log = self.log;
        stream::once(TcpListener::bind((Ipv4Addr::LOCALHOST, frontend::PORT)))
            .and_then(move |listener| future::ok(stream::try_unfold(listener, move |listener| async move {
                let (mut tcp_stream, _) = listener.accept().await?;
                frontend::PROTOCOL_VERSION.write(&mut tcp_stream).await?;
                let client_version = u8::read(&mut tcp_stream).await?;
                if client_version != frontend::PROTOCOL_VERSION {
                    return Err(Error::VersionMismatch { version: client_version, frontend })
                }
                let (reader, writer) = tcp_stream.into_split();
                let reader = LoggingReader { context: "from frontend", inner: reader, log };
                Ok(Some((
                    stream::once(future::ok(Message::FrontendConnected(FrontendWriter::Tcp(Arc::new(Mutex::new(writer))))))
                    .chain(stream::try_unfold(reader, |mut reader| async move {
                        Ok(Some((Message::Plugin(Box::new(reader.read::<frontend::ClientMessage>().await?)), reader)))
                    })),
                    listener,
                )))
            })))
            .try_flatten()
            .try_flatten()
            .map(|res| res.unwrap_or_else(|e| Message::FrontendSubscriptionError(Arc::new(e))))
            .chain(stream::pending())
            .boxed()
    }
}

pub(crate) struct Client {
    pub(crate) log: bool,
    pub(crate) websocket_url: Url,
}

impl Recipe for Client {
    type Output = Message;

    fn hash(&self, state: &mut subscription::Hasher) {
        TypeId::of::<Self>().hash(state);
    }

    fn stream(self: Box<Self>, _: EventStream) -> Pin<Box<dyn Stream<Item = Message> + Send>> {
        let log = self.log;
        stream::once(async move {
            let request = tungstenite::ClientRequestBuilder::new(self.websocket_url.to_string().try_into()?)
                .with_header(tungstenite::http::header::USER_AGENT.to_string(), user_agent());
            let (websocket, _) = tokio_tungstenite::connect_async(request).await?;
            let (sink, stream) = websocket.split();
            let stream = LoggingStream { context: "from server", inner: stream, log };
            let sink = Arc::new(Mutex::new(sink));
            let interval = interval_at(Instant::now() + Duration::from_secs(30), Duration::from_secs(30));
            Ok::<_, Error>(
                stream::once(future::ok(Message::ServerConnected(sink.clone())))
                .chain(stream::try_unfold((stream, sink, interval), |(stream, sink, mut interval)| async move {
                    let mut read = pin!(timeout(Duration::from_secs(60), stream.read_owned()));
                    Ok(loop {
                        select! {
                            res = &mut read => {
                                let (stream, msg) = res??;
                                break Some((Message::Server(msg), (stream, sink, interval)))
                            },
                            _ = interval.tick() => lock!(sink = sink; ws::ClientMessage::Ping.write_ws024(&mut *sink).await)?,
                        }
                    })
                }))
            )
        })
            .try_flatten()
            .map(|res| res.unwrap_or_else(|e| Message::ServerSubscriptionError(Arc::new(e))))
            .chain(stream::pending())
            .boxed()
    }
}
