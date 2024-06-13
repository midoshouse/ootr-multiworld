use {
    async_proto::Protocol,
    async_trait::async_trait,
    futures::stream::{
        SplitSink,
        SplitStream,
    },
    rocket_ws::WebSocket,
};
multiworld_derive::latest!();

pub mod unversioned;
pub mod v14;
pub mod v15;
pub mod v16;

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
    /// You sent the wrong password for the given room.
    #[error("wrong password")]
    WrongPassword,
    /// You attempted to create a room with a duplicate name.
    #[error("a room with this name already exists")]
    RoomExists,
    /// You tried to sign in with a Discord account that's not associated with a Mido's House account.
    #[error("no Mido's House user associated with this Discord account")]
    NoMidosHouseAccountDiscord,
    /// You tried to sign in with a racetime.gg account that's not associated with a Mido's House account.
    #[error("no Mido's House user associated with this racetime.gg account")]
    NoMidosHouseAccountRaceTime,
    /// You tried to sign in with an expired Discord session token.
    #[error("this Discord session token has expired")]
    SessionExpiredDiscord,
    /// You tried to sign in with an expired racetime.gg session token.
    #[error("this racetime.gg session token has expired")]
    SessionExpiredRaceTime,
}

impl crate::ClientKind for WebSocket {
    type SessionId = usize;
    type Reader = VersionedReader;
    type Writer = VersionedWriter;
}

#[derive(Clone, Copy)]
pub enum Version {
    V14,
    V15,
    V16,
}

pub struct VersionedReader {
    pub inner: SplitStream<rocket_ws::stream::DuplexStream>,
    pub version: Version,
}

#[async_trait]
impl crate::ClientReader for VersionedReader {
    async fn read_owned(self) -> Result<(Self, unversioned::ClientMessage), async_proto::ReadError> {
        match self.version {
            Version::V14 => v14::read_owned(self.inner).await.map(|(inner, msg)| (Self { version: Version::V14, inner }, msg)),
            Version::V15 => v15::read_owned(self.inner).await.map(|(inner, msg)| (Self { version: Version::V15, inner }, msg)),
            Version::V16 => v16::read_owned(self.inner).await.map(|(inner, msg)| (Self { version: Version::V16, inner }, msg)),
        }
    }
}

pub struct VersionedWriter {
    pub inner: SplitSink<rocket_ws::stream::DuplexStream, rocket_ws::Message>,
    pub version: Version,
}

#[async_trait]
impl crate::ClientWriter for VersionedWriter {
    async fn write(&mut self, msg: unversioned::ServerMessage) -> Result<(), async_proto::WriteError> {
        match self.version {
            Version::V14 => v14::write(&mut self.inner, msg).await,
            Version::V15 => v15::write(&mut self.inner, msg).await,
            Version::V16 => v16::write(&mut self.inner, msg).await,
        }
    }
}
