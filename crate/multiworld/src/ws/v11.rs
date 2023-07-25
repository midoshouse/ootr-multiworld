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
    either::Either,
    futures::{
        Sink,
        stream::Stream,
    },
    ootr_utils::spoiler::{
        HashIcon,
        SpoilerLog,
    },
    semver::Version,
    tokio_tungstenite::tungstenite,
    crate::{
        Filename,
        Player,
        ws::{
            ServerError,
            unversioned,
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

impl TryFrom<ClientMessage> for unversioned::ClientMessage {
    type Error = async_proto::ReadError;

    fn try_from(msg: ClientMessage) -> Result<Self, async_proto::ReadError> {
        Ok(match msg {
            ClientMessage::Ping => unversioned::ClientMessage::Ping,
            ClientMessage::JoinRoom { name, password } => unversioned::ClientMessage::JoinRoom { room: Either::Right(name), password },
            ClientMessage::CreateRoom { name, password } => unversioned::ClientMessage::CreateRoom { name, password },
            ClientMessage::Login { .. } => return Err(async_proto::ReadError::Custom(format!("ClientMessage::Login is retired. Sign in with a Mido's House API key instead."))),
            ClientMessage::Stop => unversioned::ClientMessage::Stop,
            ClientMessage::PlayerId(world) => unversioned::ClientMessage::PlayerId(world),
            ClientMessage::ResetPlayerId => unversioned::ClientMessage::ResetPlayerId,
            ClientMessage::PlayerName(filename) => unversioned::ClientMessage::PlayerName(filename),
            ClientMessage::SendItem { key, kind, target_world } => unversioned::ClientMessage::SendItem { key, kind, target_world },
            ClientMessage::KickPlayer(world) => unversioned::ClientMessage::KickPlayer(world),
            ClientMessage::DeleteRoom => unversioned::ClientMessage::DeleteRoom,
            ClientMessage::Track { mw_room_name, tracker_room_name, world_count } => unversioned::ClientMessage::Track { mw_room: Either::Right(mw_room_name), tracker_room_name, world_count },
            ClientMessage::SaveData(save) => unversioned::ClientMessage::SaveData(save),
            ClientMessage::SendAll { source_world, spoiler_log } => unversioned::ClientMessage::SendAll { source_world, spoiler_log },
            ClientMessage::SaveDataError { debug, version } => unversioned::ClientMessage::SaveDataError { debug, version },
            ClientMessage::FileHash(hash) => unversioned::ClientMessage::FileHash(hash),
            ClientMessage::AutoDeleteDelta(delta) => unversioned::ClientMessage::AutoDeleteDelta(delta),
            ClientMessage::WaitUntilEmpty => unversioned::ClientMessage::WaitUntilEmpty,
        })
    }
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
    ProgressiveItems {
        world: NonZeroU8,
        state: u32,
    },
}

impl From<unversioned::ServerMessage> for Option<ServerMessage> {
    fn from(msg: unversioned::ServerMessage) -> Self {
        match msg {
            unversioned::ServerMessage::Ping => Some(ServerMessage::Ping),
            unversioned::ServerMessage::StructuredError(e) => Some(ServerMessage::StructuredError(e)),
            unversioned::ServerMessage::OtherError(e) => Some(ServerMessage::OtherError(e)),
            unversioned::ServerMessage::EnterLobby { rooms } => Some(ServerMessage::EnterLobby { rooms: rooms.into_values().map(|(name, _)| name).collect() }),
            unversioned::ServerMessage::NewRoom { name, id: _, password_required: _ } => Some(ServerMessage::NewRoom(name)),
            unversioned::ServerMessage::DeleteRoom { name, id: _ } => Some(ServerMessage::DeleteRoom(name)),
            unversioned::ServerMessage::EnterRoom { players, num_unassigned_clients, autodelete_delta, room_id: _, allow_send_all: _ } => Some(ServerMessage::EnterRoom { players, num_unassigned_clients, autodelete_delta }),
            unversioned::ServerMessage::PlayerId(world) => Some(ServerMessage::PlayerId(world)),
            unversioned::ServerMessage::ResetPlayerId(world) => Some(ServerMessage::ResetPlayerId(world)),
            unversioned::ServerMessage::ClientConnected => Some(ServerMessage::ClientConnected),
            unversioned::ServerMessage::PlayerDisconnected(world) => Some(ServerMessage::PlayerDisconnected(world)),
            unversioned::ServerMessage::UnregisteredClientDisconnected => Some(ServerMessage::UnregisteredClientDisconnected),
            unversioned::ServerMessage::PlayerName(world, filename) => Some(ServerMessage::PlayerName(world, filename)),
            unversioned::ServerMessage::ItemQueue(items) => Some(ServerMessage::ItemQueue(items)),
            unversioned::ServerMessage::GetItem(item) => Some(ServerMessage::GetItem(item)),
            unversioned::ServerMessage::AdminLoginSuccess { active_connections } => Some(ServerMessage::AdminLoginSuccess { active_connections: active_connections.into_values().map(|(name, players, num_unassigned_clients)| (name, (players, num_unassigned_clients))).collect() }),
            unversioned::ServerMessage::Goodbye => Some(ServerMessage::Goodbye),
            unversioned::ServerMessage::PlayerFileHash(world, hash) => Some(ServerMessage::PlayerFileHash(world, hash)),
            unversioned::ServerMessage::AutoDeleteDelta(delta) => Some(ServerMessage::AutoDeleteDelta(delta)),
            unversioned::ServerMessage::RoomsEmpty => Some(ServerMessage::RoomsEmpty),
            unversioned::ServerMessage::WrongFileHash { server, client } => Some(ServerMessage::WrongFileHash { server, client }),
            unversioned::ServerMessage::ProgressiveItems { world, state } => Some(ServerMessage::ProgressiveItems { world, state }),
            unversioned::ServerMessage::LoginSuccess => unreachable!(), // old admin login system no longer works
        }
    }
}

pub(crate) async fn read_owned<R: Stream<Item = Result<tungstenite::Message, tungstenite::Error>> + Unpin + Send + 'static>(reader: R) -> Result<(R, unversioned::ClientMessage), async_proto::ReadError> {
    let (reader, msg) = ClientMessage::read_ws_owned(reader).await?;
    Ok((reader, msg.try_into()?))
}

pub(crate) async fn write(writer: &mut (impl Sink<tungstenite::Message, Error = tungstenite::Error> + Unpin + Send), msg: unversioned::ServerMessage) -> Result<(), async_proto::WriteError> {
    if let Some(msg) = Option::<ServerMessage>::from(msg) {
        msg.write_ws(writer).await?;
    }
    Ok(())
}
