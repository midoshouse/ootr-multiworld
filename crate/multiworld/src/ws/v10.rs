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
        ws::{
            ServerError,
            latest,
        },
    },
};

#[derive(Debug, Protocol)]
pub enum ClientMessage {
    Ping,
    JoinRoom {
        name: String,
        password: Option<String>,
    },
    CreateRoom {
        name: String,
        password: String,
    },
    Login {
        id: u64,
        api_key: [u8; 32],
    },
    Stop,
    PlayerId(NonZeroU8),
    ResetPlayerId,
    PlayerName(Filename),
    SendItem {
        key: u32,
        kind: u16,
        target_world: NonZeroU8,
    },
    KickPlayer(NonZeroU8),
    DeleteRoom,
    Track {
        mw_room_name: String,
        tracker_room_name: String,
        world_count: NonZeroU8,
    },
    SaveData(oottracker::Save),
    SendAll {
        source_world: NonZeroU8,
        spoiler_log: SpoilerLog,
    },
    SaveDataError {
        debug: String,
        version: Version,
    },
    FileHash([HashIcon; 5]),
    AutoDeleteDelta(Duration),
    WaitUntilEmpty,
}

#[derive(Debug, Clone, Protocol)]
pub enum ServerMessage {
    Ping,
    StructuredError(ServerError),
    OtherError(String),
    EnterLobby {
        rooms: BTreeSet<String>,
    },
    NewRoom(String),
    DeleteRoom(String),
    EnterRoom {
        players: Vec<Player>,
        num_unassigned_clients: u8,
        autodelete_delta: Duration,
    },
    PlayerId(NonZeroU8),
    ResetPlayerId(NonZeroU8),
    ClientConnected,
    PlayerDisconnected(NonZeroU8),
    UnregisteredClientDisconnected,
    PlayerName(NonZeroU8, Filename),
    ItemQueue(Vec<u16>),
    GetItem(u16),
    AdminLoginSuccess {
        active_connections: BTreeMap<String, (Vec<Player>, u8)>,
    },
    Goodbye,
    PlayerFileHash(NonZeroU8, [HashIcon; 5]),
    AutoDeleteDelta(Duration),
    RoomsEmpty,
    WrongFileHash {
        server: [HashIcon; 5],
        client: [HashIcon; 5],
    },
}

pub(super) struct ClientReader(pub(super) SplitStream<rocket_ws::stream::DuplexStream>);

#[async_trait]
impl crate::ClientReader for ClientReader {
    async fn read_owned(self) -> Result<(Self, latest::ClientMessage), async_proto::ReadError> {
        let (inner, msg) = ClientMessage::read_ws_owned(self.0).await?;
        Ok((Self(inner), match msg {
            ClientMessage::Ping => latest::ClientMessage::Ping,
            ClientMessage::JoinRoom { name, password } => latest::ClientMessage::JoinRoom { name, password },
            ClientMessage::CreateRoom { name, password } => latest::ClientMessage::CreateRoom { name, password },
            ClientMessage::Login { id, api_key } => latest::ClientMessage::Login { id, api_key },
            ClientMessage::Stop => latest::ClientMessage::Stop,
            ClientMessage::PlayerId(world) => latest::ClientMessage::PlayerId(world),
            ClientMessage::ResetPlayerId => latest::ClientMessage::ResetPlayerId,
            ClientMessage::PlayerName(filename) => latest::ClientMessage::PlayerName(filename),
            ClientMessage::SendItem { key, kind, target_world } => latest::ClientMessage::SendItem { key, kind, target_world },
            ClientMessage::KickPlayer(world) => latest::ClientMessage::KickPlayer(world),
            ClientMessage::DeleteRoom => latest::ClientMessage::DeleteRoom,
            ClientMessage::Track { mw_room_name, tracker_room_name, world_count } => latest::ClientMessage::Track { mw_room_name, tracker_room_name, world_count },
            ClientMessage::SaveData(save) => latest::ClientMessage::SaveData(save),
            ClientMessage::SendAll { source_world, spoiler_log } => latest::ClientMessage::SendAll { source_world, spoiler_log },
            ClientMessage::SaveDataError { debug, version } => latest::ClientMessage::SaveDataError { debug, version },
            ClientMessage::FileHash(hash) => latest::ClientMessage::FileHash(hash),
            ClientMessage::AutoDeleteDelta(delta) => latest::ClientMessage::AutoDeleteDelta(delta),
            ClientMessage::WaitUntilEmpty => latest::ClientMessage::WaitUntilEmpty,
        }))
    }
}

pub(super) struct ClientWriter<'a>(pub(super) &'a mut SplitSink<rocket_ws::stream::DuplexStream, rocket_ws::Message>);

#[async_trait]
impl<'a> crate::ClientWriter for ClientWriter<'a> {
    async fn write(&mut self, msg: latest::ServerMessage) -> Result<(), async_proto::WriteError> {
        let msg = match msg {
            latest::ServerMessage::Ping => Some(ServerMessage::Ping),
            latest::ServerMessage::StructuredError(e) => Some(ServerMessage::StructuredError(e)),
            latest::ServerMessage::OtherError(e) => Some(ServerMessage::OtherError(e)),
            latest::ServerMessage::EnterLobby { rooms } => Some(ServerMessage::EnterLobby { rooms }),
            latest::ServerMessage::NewRoom(name) => Some(ServerMessage::NewRoom(name)),
            latest::ServerMessage::DeleteRoom(name) => Some(ServerMessage::DeleteRoom(name)),
            latest::ServerMessage::EnterRoom { players, num_unassigned_clients, autodelete_delta } => Some(ServerMessage::EnterRoom { players, num_unassigned_clients, autodelete_delta }),
            latest::ServerMessage::PlayerId(world) => Some(ServerMessage::PlayerId(world)),
            latest::ServerMessage::ResetPlayerId(world) => Some(ServerMessage::ResetPlayerId(world)),
            latest::ServerMessage::ClientConnected => Some(ServerMessage::ClientConnected),
            latest::ServerMessage::PlayerDisconnected(world) => Some(ServerMessage::PlayerDisconnected(world)),
            latest::ServerMessage::UnregisteredClientDisconnected => Some(ServerMessage::UnregisteredClientDisconnected),
            latest::ServerMessage::PlayerName(world, filename) => Some(ServerMessage::PlayerName(world, filename)),
            latest::ServerMessage::ItemQueue(items) => Some(ServerMessage::ItemQueue(items)),
            latest::ServerMessage::GetItem(item) => Some(ServerMessage::GetItem(item)),
            latest::ServerMessage::AdminLoginSuccess { active_connections } => Some(ServerMessage::AdminLoginSuccess { active_connections }),
            latest::ServerMessage::Goodbye => Some(ServerMessage::Goodbye),
            latest::ServerMessage::PlayerFileHash(world, hash) => Some(ServerMessage::PlayerFileHash(world, hash)),
            latest::ServerMessage::AutoDeleteDelta(delta) => Some(ServerMessage::AutoDeleteDelta(delta)),
            latest::ServerMessage::RoomsEmpty => Some(ServerMessage::RoomsEmpty),
            latest::ServerMessage::WrongFileHash { server, client } => Some(ServerMessage::WrongFileHash { server, client }),
            latest::ServerMessage::ProgressiveItems { .. } => None, // optional feature not supported by the client, ignore
        };
        if let Some(msg) = msg {
            msg.write_ws(self.0).await?;
        }
        Ok(())
    }
}
