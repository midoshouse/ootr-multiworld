use {
    std::{
        any::TypeId,
        hash::{
            Hash as _,
            Hasher,
        },
        net::Ipv4Addr,
        num::NonZeroU8,
        sync::Arc,
        time::Duration,
    },
    async_proto::Protocol,
    futures::{
        future,
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
    multiworld::{
        Filename,
        HashIcon,
    },
    crate::{
        Error,
        MW_PJ64_PROTO_VERSION,
        Message,
    },
};

#[derive(Debug, Protocol)]
pub(crate) enum ServerMessage {
    ItemQueue(Vec<u16>),
    GetItem(u16),
    PlayerName(NonZeroU8, Filename),
}

#[derive(Debug, Clone, Protocol)]
pub(crate) enum ClientMessage {
    PlayerId(NonZeroU8),
    PlayerName(Filename),
    SendItem {
        key: u32,
        kind: u16,
        target_world: NonZeroU8,
    },
    SaveData([u8; oottracker::save::SIZE]),
    FileHash([HashIcon; 5]),
}

pub(crate) struct Pj64Listener(pub(crate) u8);

impl<H: Hasher, I> Recipe<H, I> for Pj64Listener {
    type Output = Message;

    fn hash(&self, state: &mut H) {
        TypeId::of::<Self>().hash(state);
        self.0.hash(state);
    }

    fn stream(self: Box<Self>, _: BoxStream<'_, I>) -> BoxStream<'_, Message> {
        stream::once(TcpListener::bind((Ipv4Addr::LOCALHOST, 24818)))
            .and_then(|listener| future::ok(stream::try_unfold(listener, |listener| async move {
                let (mut tcp_stream, _) = listener.accept().await?;
                MW_PJ64_PROTO_VERSION.write(&mut tcp_stream).await?;
                let client_version = u8::read(&mut tcp_stream).await?;
                if client_version != MW_PJ64_PROTO_VERSION { return Err(Error::VersionMismatch(client_version)) }
                let (reader, writer) = tcp_stream.into_split();
                Ok(Some((
                    stream::once(future::ok(Message::Pj64Connected(Arc::new(Mutex::new(writer)))))
                    .chain(stream::try_unfold(reader, |mut reader| async move {
                        Ok(Some((Message::Plugin(ClientMessage::read(&mut reader).await?), reader)))
                    })),
                    listener,
                )))
            })))
            .try_flatten()
            .try_flatten()
            .map(|res| match res {
                Ok(msg) => msg,
                Err(e) => Message::Pj64SubscriptionError(Arc::new(e)),
            })
            .boxed()
    }
}

pub(crate) struct Client(pub(crate) u16);

impl<H: Hasher, I> Recipe<H, I> for Client {
    type Output = Message;

    fn hash(&self, state: &mut H) {
        TypeId::of::<Self>().hash(state);
    }

    fn stream(self: Box<Self>, _: BoxStream<'_, I>) -> BoxStream<'_, Message> {
        stream::once(TcpStream::connect((multiworld::ADDRESS_V4, self.0)))
            .err_into::<Error>()
            .and_then(|mut tcp_stream| async move {
                multiworld::handshake(&mut tcp_stream).await?;
                let (reader, writer) = tcp_stream.into_split();
                let writer = Arc::new(Mutex::new(writer));
                let interval = interval_at(Instant::now() + Duration::from_secs(30), Duration::from_secs(30));
                Ok(
                    stream::once(future::ok(Message::ServerConnected(writer.clone())))
                    .chain(stream::try_unfold((reader, writer, interval), |(reader, writer, mut interval)| async move {
                        pin! {
                            let read = timeout(Duration::from_secs(60), multiworld::ServerMessage::read_owned(reader));
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
            .boxed()
    }
}
