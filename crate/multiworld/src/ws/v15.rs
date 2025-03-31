use {
    std::{
        collections::BTreeMap,
        num::NonZeroU8,
        time::Duration,
    },
    async_proto::Protocol,
    chrono::prelude::*,
    futures::{
        Sink,
        stream::Stream,
    },
    ootr::model::DungeonReward,
    ootr_utils::spoiler::HashIcon,
    semver::Version,
    tokio_tungstenite::tungstenite,
    crate::{
        Filename,
        HintArea,
        Player,
        ws::{
            ServerError,
            unversioned,
            v16::SpoilerLog,
        },
    },
};

#[derive(Debug, Protocol)]
pub enum ClientMessage {
    Ping,
    JoinRoom {
        id: u64,
        password: Option<String>,
    },
    CreateRoom {
        name: String,
        password: String,
    },
    LoginApiKey {
        api_key: String,
    },
    Stop,
    PlayerId(NonZeroU8),
    ResetPlayerId,
    PlayerName(Filename),
    SendItem {
        key: u64,
        kind: u16,
        target_world: NonZeroU8,
    },
    KickPlayer(NonZeroU8),
    DeleteRoom,
    Track {
        mw_room: u64,
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
    LoginDiscord {
        bearer_token: String,
    },
    LoginRaceTime {
        bearer_token: String,
    },
    LeaveRoom,
    DungeonRewardInfo {
        reward: DungeonReward,
        world: NonZeroU8,
        area: HintArea,
    },
}

impl TryFrom<ClientMessage> for unversioned::ClientMessage {
    type Error = async_proto::ReadError;

    fn try_from(msg: ClientMessage) -> Result<Self, async_proto::ReadError> {
        Ok(match msg {
            ClientMessage::Ping => unversioned::ClientMessage::Ping,
            ClientMessage::JoinRoom { id, password } => unversioned::ClientMessage::JoinRoom { id, password },
            ClientMessage::CreateRoom { name, password } => unversioned::ClientMessage::CreateRoom { name, password },
            ClientMessage::LoginApiKey { api_key } => unversioned::ClientMessage::LoginApiKey { api_key },
            ClientMessage::Stop => unversioned::ClientMessage::Stop,
            ClientMessage::PlayerId(world) => unversioned::ClientMessage::PlayerId(world),
            ClientMessage::ResetPlayerId => unversioned::ClientMessage::ResetPlayerId,
            ClientMessage::PlayerName(filename) => unversioned::ClientMessage::PlayerName(filename),
            ClientMessage::SendItem { key, kind, target_world } => unversioned::ClientMessage::SendItem { key, kind, target_world },
            ClientMessage::KickPlayer(world) => unversioned::ClientMessage::KickPlayer(world),
            ClientMessage::DeleteRoom => unversioned::ClientMessage::DeleteRoom,
            ClientMessage::Track { mw_room, tracker_room_name, world_count } => unversioned::ClientMessage::Track { mw_room, tracker_room_name, world_count },
            ClientMessage::SaveData(save) => unversioned::ClientMessage::SaveData(save),
            ClientMessage::SendAll { source_world, spoiler_log } => unversioned::ClientMessage::SendAll { source_world, spoiler_log: spoiler_log.into() },
            ClientMessage::SaveDataError { debug, version } => unversioned::ClientMessage::SaveDataError { debug, version },
            ClientMessage::FileHash(hash) => unversioned::ClientMessage::FileHash(Some(hash)),
            ClientMessage::AutoDeleteDelta(delta) => unversioned::ClientMessage::AutoDeleteDelta(delta),
            ClientMessage::WaitUntilEmpty => unversioned::ClientMessage::WaitUntilEmpty,
            ClientMessage::LoginDiscord { bearer_token } => unversioned::ClientMessage::LoginDiscord { bearer_token },
            ClientMessage::LoginRaceTime { bearer_token } => unversioned::ClientMessage::LoginRaceTime { bearer_token },
            ClientMessage::LeaveRoom => unversioned::ClientMessage::LeaveRoom,
            ClientMessage::DungeonRewardInfo { reward, world, area } => unversioned::ClientMessage::DungeonRewardInfo { reward, world, area },
        })
    }
}

#[derive(Debug, Clone, Protocol)]
pub enum ServerMessage {
    Ping,
    StructuredError(ServerError),
    OtherError(String),
    EnterLobby {
        rooms: BTreeMap<u64, (String, bool)>,
    },
    NewRoom {
        id: u64,
        name: String,
        password_required: bool,
    },
    DeleteRoom(u64),
    EnterRoom {
        room_id: u64,
        players: Vec<Player>,
        num_unassigned_clients: u8,
        autodelete_delta: Duration,
        allow_send_all: bool,
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
        active_connections: BTreeMap<u64, (Vec<Player>, u8)>,
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
    LoginSuccess,
    WorldTaken(NonZeroU8),
    WorldFreed,
    MaintenanceNotice {
        start: DateTime<Utc>,
        duration: Duration,
    },
}

impl From<unversioned::ServerMessage> for Option<ServerMessage> {
    fn from(msg: unversioned::ServerMessage) -> Self {
        match msg {
            unversioned::ServerMessage::Ping => Some(ServerMessage::Ping),
            unversioned::ServerMessage::StructuredError(e) => Some(ServerMessage::StructuredError(e)),
            unversioned::ServerMessage::OtherError(e) => Some(ServerMessage::OtherError(e)),
            unversioned::ServerMessage::EnterLobby { rooms } => Some(ServerMessage::EnterLobby { rooms }),
            unversioned::ServerMessage::NewRoom { id, name, password_required } => Some(ServerMessage::NewRoom { id, name, password_required }),
            unversioned::ServerMessage::DeleteRoom(id) => Some(ServerMessage::DeleteRoom(id)),
            unversioned::ServerMessage::EnterRoom { room_id, players, num_unassigned_clients, autodelete_delta, allow_send_all } => Some(ServerMessage::EnterRoom { room_id, players, num_unassigned_clients, autodelete_delta, allow_send_all }),
            unversioned::ServerMessage::PlayerId(world) => Some(ServerMessage::PlayerId(world)),
            unversioned::ServerMessage::ResetPlayerId(world) => Some(ServerMessage::ResetPlayerId(world)),
            unversioned::ServerMessage::ClientConnected => Some(ServerMessage::ClientConnected),
            unversioned::ServerMessage::PlayerDisconnected(world) => Some(ServerMessage::PlayerDisconnected(world)),
            unversioned::ServerMessage::UnregisteredClientDisconnected => Some(ServerMessage::UnregisteredClientDisconnected),
            unversioned::ServerMessage::PlayerName(world, filename) => Some(ServerMessage::PlayerName(world, filename)),
            unversioned::ServerMessage::ItemQueue(items) => Some(ServerMessage::ItemQueue(items)),
            unversioned::ServerMessage::GetItem(item) => Some(ServerMessage::GetItem(item)),
            unversioned::ServerMessage::AdminLoginSuccess { active_connections } => Some(ServerMessage::AdminLoginSuccess { active_connections }),
            unversioned::ServerMessage::Goodbye => Some(ServerMessage::Goodbye),
            unversioned::ServerMessage::PlayerFileHash(world, hash) => Some(ServerMessage::PlayerFileHash(world, hash?)),
            unversioned::ServerMessage::AutoDeleteDelta(delta) => Some(ServerMessage::AutoDeleteDelta(delta)),
            unversioned::ServerMessage::RoomsEmpty => Some(ServerMessage::RoomsEmpty),
            unversioned::ServerMessage::WrongFileHash { server, client } => Some(if let (Some(server), Some(client)) = (server, client) {
                ServerMessage::WrongFileHash { server, client }
            } else {
                ServerMessage::OtherError(format!("this room is for a different seed: server has {} but client has {}", crate::format_opt_hash(server), crate::format_opt_hash(client)))
            }),
            unversioned::ServerMessage::ProgressiveItems { world, state } => Some(ServerMessage::ProgressiveItems { world, state }),
            unversioned::ServerMessage::LoginSuccess => Some(ServerMessage::LoginSuccess),
            unversioned::ServerMessage::WorldTaken(world) => Some(ServerMessage::WorldTaken(world)),
            unversioned::ServerMessage::WorldFreed => Some(ServerMessage::WorldFreed),
            unversioned::ServerMessage::MaintenanceNotice { start, duration } => Some(ServerMessage::MaintenanceNotice { start, duration }),
        }
    }
}

pub(crate) async fn read_owned<R: Stream<Item = Result<tungstenite::Message, tungstenite::Error>> + Unpin + Send + 'static>(reader: R) -> Result<(R, unversioned::ClientMessage), async_proto::ReadError> {
    let (reader, msg) = ClientMessage::read_ws_owned021(reader).await?;
    Ok((reader, msg.try_into()?))
}

pub(crate) async fn write(writer: &mut (impl Sink<tungstenite::Message, Error = tungstenite::Error> + Unpin + Send), msg: unversioned::ServerMessage) -> Result<(), async_proto::WriteError> {
    if let Some(msg) = Option::<ServerMessage>::from(msg) {
        msg.write_ws021(writer).await?;
    }
    Ok(())
}
