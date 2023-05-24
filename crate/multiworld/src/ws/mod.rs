use {
    async_proto::Protocol,
    async_trait::async_trait,
    futures::stream::{
        SplitSink,
        SplitStream,
    },
    rocket_ws::WebSocket,
};
pub use self::v11 as latest;

pub mod v10;
pub mod v11;

macro_rules! server_errors {
    ($($(#[$attr:meta])* $variant:ident),* $(,)?) => {
        /// New unit variants on this enum don't cause a major version bump, since the client interprets them as instances of the `Future` variant.
        #[derive(Debug, Clone, Copy, Protocol, thiserror::Error)]
        #[async_proto(via = u8, clone)]
        pub enum ServerError {
            /// The server sent a `ServerError` that the client doesn't know about yet.
            #[error("server error #{0}")]
            Future(u8),
            $($(#[$attr])* $variant,)*
        }

        impl From<u8> for ServerError {
            fn from(discrim: u8) -> Self {
                let iter_discrim = 1;
                $(
                    if discrim == iter_discrim { return Self::$variant }
                    #[allow(unused)] let iter_discrim = iter_discrim + 1;
                )*
                Self::Future(discrim)
            }
        }

        impl From<ServerError> for u8 {
            fn from(e: ServerError) -> Self {
                if let ServerError::Future(discrim) = e { return discrim }
                let iter_discrim = 1u8;
                $(
                    if let ServerError::$variant = e { return iter_discrim }
                    #[allow(unused)] let iter_discrim = iter_discrim + 1;
                )*
                unreachable!()
            }
        }
    };
}

server_errors! {
    /// The client sent the wrong password for the given room.
    #[error("wrong password")]
    WrongPassword,
    /// The client attempted to create a room with a duplicate name.
    #[error("a room with this name already exists")]
    RoomExists,
}

impl crate::ClientKind for WebSocket {
    type SessionId = usize;
    type Reader = VersionedReader;
    type Writer = VersionedWriter;
}

#[derive(Clone, Copy)]
pub enum Version {
    V10,
    V11,
}

pub struct VersionedReader {
    pub inner: SplitStream<rocket_ws::stream::DuplexStream>,
    pub version: Version,
}

#[async_trait]
impl crate::ClientReader for VersionedReader {
    async fn read_owned(self) -> Result<(Self, latest::ClientMessage), async_proto::ReadError> {
        match self.version {
            Version::V10 => v10::ClientReader(self.inner).read_owned().await.map(|(v10::ClientReader(inner), msg)| (Self { version: Version::V10, inner }, msg)),
            Version::V11 => v11::ClientReader(self.inner).read_owned().await.map(|(v11::ClientReader(inner), msg)| (Self { version: Version::V10, inner }, msg)),
        }
    }
}

pub struct VersionedWriter {
    pub inner: SplitSink<rocket_ws::stream::DuplexStream, rocket_ws::Message>,
    pub version: Version,
}

#[async_trait]
impl crate::ClientWriter for VersionedWriter {
    async fn write(&mut self, msg: latest::ServerMessage) -> Result<(), async_proto::WriteError> {
        match self.version {
            Version::V10 => v10::ClientWriter(&mut self.inner).write(msg).await,
            Version::V11 => v11::ClientWriter(&mut self.inner).write(msg).await,
        }
    }
}
