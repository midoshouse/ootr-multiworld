use {
    std::{
        collections::{
            BTreeMap,
            BTreeSet,
        },
        num::NonZeroU8,
        time::Duration,
    },
    async_proto::Protocol,
    async_trait::async_trait,
    futures::stream::{
        SplitSink,
        SplitStream,
    },
    ootr_utils::spoiler::{
        HashIcon,
        SpoilerLog,
    },
    semver::Version,
    crate::{
        Filename,
        Player,
        ws::ServerError,
    },
};

#[derive(Debug, Protocol)]
pub enum ClientMessage {
    /// Tells the server we're still here. Should be sent every 30 seconds; the server will consider the connection lost if no message is received for 60 seconds.
    Ping,
    /// Only works after [`ServerMessage::EnterLobby`].
    JoinRoom {
        name: String,
        password: Option<String>,
    },
    /// Only works after [`ServerMessage::EnterLobby`].
    CreateRoom {
        name: String,
        password: String,
    },
    /// Sign in with a Mido's House API key. Currently only available for Mido's House admins. Only works after [`ServerMessage::EnterLobby`].
    Login {
        id: u64,
        api_key: [u8; 32],
    },
    /// Stops the server. Only works after [`ServerMessage::AdminLoginSuccess`].
    Stop,
    /// Claims a world. Only works after [`ServerMessage::EnterRoom`].
    PlayerId(NonZeroU8),
    /// Unloads the previously claimed world. Only works after [`ServerMessage::EnterRoom`].
    ResetPlayerId,
    /// Player names are encoded in the NTSC charset, with trailing spaces (`0xdf`). Only works after [`ServerMessage::PlayerId`].
    PlayerName(Filename),
    /// Only works after [`ServerMessage::EnterRoom`].
    SendItem {
        key: u32,
        kind: u16,
        target_world: NonZeroU8,
    },
    /// Only works after [`ServerMessage::EnterRoom`].
    KickPlayer(NonZeroU8),
    /// Only works after [`ServerMessage::EnterRoom`].
    DeleteRoom,
    /// Configures the given room to be visible on oottracker.fenhl.net. Only works after [`ServerMessage::AdminLoginSuccess`].
    Track {
        mw_room_name: String,
        tracker_room_name: String, //TODO remove this parameter, generate a random name instead and reply with it
        world_count: NonZeroU8, //TODO this parameter can also be removed if oottracker is changed to use the base queue system
    },
    /// Only works after [`ServerMessage::EnterRoom`].
    SaveData(oottracker::Save),
    /// Sends all remaining items from the given world to the given room. Only works after [`ServerMessage::EnterRoom`].
    SendAll {
        source_world: NonZeroU8,
        spoiler_log: SpoilerLog,
    },
    /// Reports an error with decoding save data.
    SaveDataError {
        debug: String,
        version: Version,
    },
    /// Reports the loaded seed's file hash icons, allowing the server to ensure that all players are on the same seed. Only works after [`ServerMessage::PlayerId`].
    FileHash([HashIcon; 5]),
    /// Sets the time after which the room should be automatically deleted. Only works after [`ServerMessage::EnterRoom`].
    AutoDeleteDelta(Duration),
    /// Requests a [`ServerMessage::RoomsEmpty`] when no players with claimed worlds are in any rooms. Only works after [`ServerMessage::AdminLoginSuccess`].
    WaitUntilEmpty,
}

#[derive(Debug, Clone, Protocol)]
pub enum ServerMessage {
    /// Tells the client we're still here. Sent every 30 seconds; clients should consider the connection lost if no message is received for 60 seconds.
    Ping,
    /// An error that the client might be able to recover from has occurred.
    StructuredError(ServerError),
    /// A fatal error has occurred. Contains a human-readable error message.
    OtherError(String),
    /// You have just connected or left a room.
    EnterLobby {
        rooms: BTreeSet<String>,
    },
    /// A new room has been created.
    NewRoom(String),
    /// A room has been deleted.
    DeleteRoom(String),
    /// You have created or joined a room.
    EnterRoom {
        players: Vec<Player>,
        num_unassigned_clients: u8,
        autodelete_delta: Duration,
    },
    /// A previously unassigned world has been taken by a client.
    PlayerId(NonZeroU8),
    /// A previously assigned world has been unassigned.
    ResetPlayerId(NonZeroU8),
    /// A new (unassigned) client has connected to the room.
    ClientConnected,
    /// A client with a world has disconnected from the room.
    PlayerDisconnected(NonZeroU8),
    /// A client without a world has disconnected from the room.
    UnregisteredClientDisconnected,
    /// A player has changed their name.
    ///
    /// Player names are encoded in the NTSC charset, with trailing spaces (`0xdf`).
    PlayerName(NonZeroU8, Filename),
    /// Your list of received items has changed.
    ItemQueue(Vec<u16>),
    /// You have received a new item, add it to the end of your item queue.
    GetItem(u16),
    /// You have logged in as an admin.
    AdminLoginSuccess {
        active_connections: BTreeMap<String, (Vec<Player>, u8)>,
    },
    /// The client will now be disconnected.
    Goodbye,
    /// A player has sent their file select hash icons.
    PlayerFileHash(NonZeroU8, [HashIcon; 5]),
    /// Sets the time after which the room will be automatically deleted has been changed.
    AutoDeleteDelta(Duration),
    /// There are no active players in any rooms. Sent after [`ClientMessage::WaitUntilEmpty`].
    RoomsEmpty,
    /// The client has the wrong seed loaded.
    WrongFileHash {
        server: [HashIcon; 5],
        client: [HashIcon; 5],
    },
    /// Updates the progressive items state for the given player.
    ProgressiveItems {
        world: NonZeroU8,
        /// Like `mw_progressive_items_state_t` in the randomizer.
        state: u32,
    },
}

pub(super) struct ClientReader(pub(super) SplitStream<rocket_ws::stream::DuplexStream>);

#[async_trait]
impl crate::ClientReader for ClientReader {
    async fn read_owned(self) -> Result<(Self, ClientMessage), async_proto::ReadError> {
        let (inner, msg) = ClientMessage::read_ws_owned(self.0).await?;
        Ok((Self(inner), msg))
    }
}

pub(super) struct ClientWriter<'a>(pub(super) &'a mut SplitSink<rocket_ws::stream::DuplexStream, rocket_ws::Message>);

#[async_trait]
impl<'a> crate::ClientWriter for ClientWriter<'a> {
    async fn write(&mut self, msg: ServerMessage) -> Result<(), async_proto::WriteError> {
        msg.write_ws(self.0).await
    }
}
