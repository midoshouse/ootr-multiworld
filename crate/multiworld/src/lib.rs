#![deny(rust_2018_idioms, unused, unused_crate_dependencies, unused_import_braces, unused_lifetimes, unused_qualifications, warnings)]
#![forbid(unsafe_code)]

use {
    std::{
        collections::{
            BTreeMap,
            BTreeSet,
            HashMap,
            HashSet,
        },
        fmt,
        hash::Hash,
        mem,
        num::NonZeroU8,
        sync::Arc,
        time::Duration,
    },
    async_proto::Protocol,
    async_recursion::async_recursion,
    async_trait::async_trait,
    chrono::prelude::*,
    futures::stream::{
        SplitSink,
        SplitStream,
    },
    itertools::Itertools as _,
    ootr_utils::spoiler::{
        HashIcon,
        SpoilerLog,
    },
    oottracker::websocket::MwItem as Item,
    rocket_ws::WebSocket,
    semver::Version,
    tokio::{
        net::{
            TcpStream,
            tcp::{
                OwnedReadHalf,
                OwnedWriteHalf,
            },
        },
        sync::{
            Mutex,
            broadcast,
            oneshot,
        },
    },
    url::Url,
    wheel::traits::IsNetworkError,
};
#[cfg(unix)] use std::os::unix::io::AsRawFd;
#[cfg(windows)] use std::os::windows::io::AsRawSocket;
#[cfg(feature = "pyo3")] use pyo3::prelude::*;
#[cfg(feature = "sqlx")] use sqlx::PgPool;

pub mod config;
pub mod frontend;
pub mod github;

pub const DEFAULT_TCP_PORT: u16 = 24809; //TODO use for LAN support

pub const CREDENTIAL_LEN: usize = ring::digest::SHA512_OUTPUT_LEN;

pub fn version() -> Version { Version::parse(env!("CARGO_PKG_VERSION")).expect("failed to parse package version") }
pub fn proto_version() -> u8 { version().major.try_into().expect("version number does not fit into u8") }
pub fn websocket_url() -> Url { Url::parse(&format!("https://mw.midos.house/v{}", version().major)).expect("failed to parse WebSocket URL") }

const TRIFORCE_PIECE: u16 = 0x00ca;

fn natjoin<T: fmt::Display>(elts: impl IntoIterator<Item = T>) -> Option<String> {
    let mut elts = elts.into_iter().fuse();
    match (elts.next(), elts.next(), elts.next()) {
        (None, _, _) => None,
        (Some(elt), None, _) => Some(elt.to_string()),
        (Some(elt1), Some(elt2), None) => Some(format!("{elt1} and {elt2}")),
        (Some(elt1), Some(elt2), Some(elt3)) => {
            let mut rest = [elt2, elt3].into_iter().chain(elts).collect_vec();
            let last = rest.pop().expect("rest contains at least elt2 and elt3");
            Some(format!("{elt1}, {}, and {last}", rest.into_iter().format(", ")))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct DurationFormatter(pub Duration);

impl fmt::Display for DurationFormatter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let secs = self.0.as_secs();

        let mins = secs / 60;
        let secs = secs % 60;

        let hours = mins / 60;
        let mins = mins % 60;

        let days = hours / 24;
        let hours = hours % 24;

        let parts = (days > 0).then(|| format!("{days} day{}", if days == 1 { "" } else { "s" })).into_iter()
            .chain((hours > 0).then(|| format!("{hours} hour{}", if hours == 1 { "" } else { "s" })))
            .chain((mins > 0).then(|| format!("{mins} minute{}", if mins == 1 { "" } else { "s" })))
            .chain((secs > 0).then(|| format!("{secs} second{}", if secs == 1 { "" } else { "s" })));
        if let Some(formatted) = natjoin(parts) {
            write!(f, "{formatted}")
        } else {
            write!(f, "0 seconds")
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Protocol)]
pub struct Filename(pub [u8; 8]);

impl Filename {
    pub const DEFAULT: Self = Self([0xdf; 8]);
    const ENCODING: [char; 256] = [
        '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'あ', 'い', 'う', 'え', 'お', 'か',
        'き', 'く', 'け', 'こ', 'さ', 'し', 'す', 'せ', 'そ', 'た', 'ち', 'つ', 'て', 'と', 'な', 'に',
        'ぬ', 'ね', 'の', 'は', 'ひ', 'ふ', 'へ', 'ほ', 'ま', 'み', 'む', 'め', 'も', 'や', 'ゆ', 'よ',
        'ら', 'り', 'る', 'れ', 'ろ', 'わ', 'を', 'ん', 'ぁ', 'ぃ', 'ぅ', 'ぇ', 'ぉ', 'っ', 'ゃ', 'ゅ',
        'ょ', 'が', 'ぎ', 'ぐ', 'げ', 'ご', 'ざ', 'じ', 'ず', 'ぜ', 'ぞ', 'だ', 'ぢ', 'づ', 'で', 'ど',
        'ば', 'び', 'ぶ', 'べ', 'ぼ', 'ぱ', 'ぴ', 'ぷ', 'ぺ', 'ぽ', 'ア', 'イ', 'ウ', 'エ', 'オ', 'カ',
        'キ', 'ク', 'ケ', 'コ', 'サ', 'シ', 'ス', 'セ', 'ソ', 'タ', 'チ', 'ツ', 'テ', 'ト', 'ナ', 'ニ',
        'ヌ', 'ネ', 'ノ', 'ハ', 'ヒ', 'フ', 'ヘ', 'ホ', 'マ', 'ミ', 'ム', 'メ', 'モ', 'ヤ', 'ユ', 'ヨ',
        'ラ', 'リ', 'ル', 'レ', 'ロ', 'ワ', 'ヲ', 'ン', 'ァ', 'ィ', 'ゥ', 'ェ', 'ォ', 'ッ', 'ャ', 'ュ',
        'ョ', 'ガ', 'ギ', 'グ', 'ゲ', 'ゴ', 'ザ', 'ジ', 'ズ', 'ゼ', 'ゾ', 'ダ', 'ヂ', 'ヅ', 'デ', 'ド',
        'バ', 'ビ', 'ブ', 'ベ', 'ボ', 'パ', 'ピ', 'プ', 'ペ', 'ポ', 'ヴ', 'A', 'B', 'C', 'D', 'E',
        'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M', 'N', 'O', 'P', 'Q', 'R', 'S', 'T', 'U',
        'V', 'W', 'X', 'Y', 'Z', 'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k',
        'l', 'm', 'n', 'o', 'p', 'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z', ' ',
        '┬', '?', '!', ':', '-', '(', ')', '゛', '゜', ',', '.', '/', '�', '�', '�', '�',
        '�', '�', '�', '�', '�', '�', '�', '�', '�', '�', '�', '�', '�', '�', '�', '�',
    ];

    pub fn fallback(world: NonZeroU8) -> Self {
        match world.get() {
            0 => unreachable!(),
            n @ 1..=9 => Self([0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, 0xdf, n]), // Player N
            n @ 10..=99 => {
                let tens = n / 10;
                let ones = n % 10;
                Self([0xba, 0xd0, 0xc5, 0xdd, 0xc9, 0xd6, tens, ones]) // PlayerNN
            }
            n @ 100..=255 => {
                let hundreds = n / 100;
                let tens = (n % 100) / 10;
                let ones = n % 10;
                Self([0xba, 0xd0, 0xc5, 0xdd, 0xd6, hundreds, tens, ones]) //PlayrNNN
            }
        }
    }
}

impl Default for Filename {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl<'a> TryFrom<&'a [u8]> for Filename {
    type Error = <[u8; 8] as TryFrom<&'a [u8]>>::Error;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        value.try_into().map(Self)
    }
}

impl fmt::Display for Filename {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for c in self.0 {
            Self::ENCODING[usize::from(c)].fmt(f)?;
        }
        Ok(())
    }
}

impl fmt::Debug for Filename {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.to_string())
    }
}

impl PartialEq<&[u8]> for Filename {
    fn eq(&self, other: &&[u8]) -> bool {
        self.0 == *other
    }
}

#[derive(Debug, Clone, Copy, Protocol)]
pub struct Player {
    pub world: NonZeroU8,
    pub name: Filename,
    file_hash: Option<[HashIcon; 5]>,
}

impl Player {
    pub fn new(world: NonZeroU8) -> Self {
        Self {
            world,
            name: Filename::default(),
            file_hash: None,
        }
    }
}

#[derive(Debug)]
pub enum EndRoomSession {
    ToLobby,
    Disconnect,
}

#[async_trait]
pub trait ClientReader: Unpin + Send + Sized + 'static {
    async fn read_owned(self) -> Result<(Self, ClientMessage), async_proto::ReadError>;
}

#[async_trait]
pub trait ClientWriter: Unpin + Send {
    async fn write(&mut self, msg: &ServerMessage) -> Result<(), async_proto::WriteError>;
}

pub trait ClientKind {
    type SessionId: fmt::Debug + Copy + Eq + Hash + Send + Sync;
    type Reader: ClientReader;
    type Writer: ClientWriter;
}

impl ClientKind for WebSocket {
    type SessionId = usize;
    type Reader = SplitStream<rocket_ws::stream::DuplexStream>;
    type Writer = SplitSink<rocket_ws::stream::DuplexStream, rocket_ws::Message>;
}

#[async_trait]
impl ClientReader for SplitStream<rocket_ws::stream::DuplexStream> {
    async fn read_owned(self) -> Result<(Self, ClientMessage), async_proto::ReadError> {
        ClientMessage::read_ws_owned(self).await
    }
}

#[async_trait]
impl ClientWriter for SplitSink<rocket_ws::stream::DuplexStream, rocket_ws::Message> {
    async fn write(&mut self, msg: &ServerMessage) -> Result<(), async_proto::WriteError> {
        msg.write_ws(self).await
    }
}

#[cfg(unix)] pub type SocketId = std::os::unix::io::RawFd;
#[cfg(windows)] pub type SocketId = std::os::windows::io::RawSocket;

#[cfg(unix)] pub fn socket_id<T: AsRawFd>(socket: &T) -> SocketId { socket.as_raw_fd() }
#[cfg(windows)] pub fn socket_id<T: AsRawSocket>(socket: &T) -> SocketId { socket.as_raw_socket() }

impl ClientKind for SocketId {
    type SessionId = Self;
    type Reader = OwnedReadHalf;
    type Writer = OwnedWriteHalf;
}

#[async_trait]
impl ClientReader for OwnedReadHalf {
    async fn read_owned(self) -> Result<(Self, ClientMessage), async_proto::ReadError> {
        ClientMessage::read_owned(self).await
    }
}

#[async_trait]
impl ClientWriter for OwnedWriteHalf {
    async fn write(&mut self, msg: &ServerMessage) -> Result<(), async_proto::WriteError> {
        msg.write(self).await
    }
}

pub struct Client<C: ClientKind> {
    pub writer: Arc<Mutex<C::Writer>>,
    pub end_tx: oneshot::Sender<EndRoomSession>,
    pub player: Option<Player>,
    pub save_data: Option<oottracker::Save>,
}

impl<C: ClientKind> fmt::Debug for Client<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self { writer: _, end_tx, player, save_data } = self;
        f.debug_struct("Client")
            .field("writer", &format_args!("_"))
            .field("end_tx", end_tx)
            .field("player", player)
            .field("save_data", save_data)
            .finish()
    }
}

pub struct Room<C: ClientKind> {
    pub name: String,
    pub password_hash: [u8; CREDENTIAL_LEN],
    pub password_salt: [u8; CREDENTIAL_LEN],
    pub clients: HashMap<C::SessionId, Client<C>>,
    pub file_hash: Option<[HashIcon; 5]>,
    pub base_queue: Vec<Item>,
    pub player_queues: HashMap<NonZeroU8, Vec<Item>>,
    pub last_saved: DateTime<Utc>,
    pub autodelete_delta: Duration,
    pub autodelete_tx: broadcast::Sender<(String, DateTime<Utc>)>,
    #[cfg(feature = "sqlx")]
    pub db_pool: PgPool,
    #[cfg(feature = "tokio-tungstenite")]
    pub tracker_state: Option<(String, tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>)>,
}

impl<C: ClientKind> fmt::Debug for Room<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self {
            password_hash: _,
            password_salt: _,
            name, clients, file_hash, base_queue, player_queues, last_saved, autodelete_delta, autodelete_tx,
            #[cfg(feature = "sqlx")] db_pool,
            #[cfg(feature = "tokio-tungstenite")] tracker_state,
        } = self;
        let mut struct_f = f.debug_struct("Room");
        struct_f.field("name", name);
        struct_f.field("password_hash", &format_args!("_"));
        struct_f.field("password_salt", &format_args!("_"));
        struct_f.field("clients", clients);
        struct_f.field("file_hash", file_hash);
        struct_f.field("base_queue", base_queue);
        struct_f.field("player_queues", player_queues);
        struct_f.field("last_saved", last_saved);
        struct_f.field("autodelete_delta", autodelete_delta);
        struct_f.field("autodelete_tx", autodelete_tx);
        #[cfg(feature = "sqlx")] struct_f.field("db_pool", db_pool);
        #[cfg(feature = "tokio-tungstenite")] struct_f.field("tracker_state", tracker_state);
        struct_f.finish()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum QueueItemError {
    #[error(transparent)] Write(#[from] async_proto::WriteError),
    #[error("this room is for a different seed")]
    FileHash,
    #[error("please claim a world before sending items")]
    NoSourceWorld,
}

#[derive(Debug, thiserror::Error)]
pub enum SetHashError {
    #[error("this room is for a different seed")]
    FileHash,
    #[error("please claim a world before reporting your file hash")]
    NoSourceWorld,
}

#[cfg(feature = "pyo3")]
#[derive(Debug, thiserror::Error)]
pub enum SendItemError {
    #[error("unknown location: {0}")]
    Key(String),
    #[error("unknown item kind: {0}")]
    Kind(String),
}

#[cfg(feature = "pyo3")]
fn display_send_item_errors(errors: &[SendItemError]) -> String {
    match errors {
        [] => format!("empty SendItemError list"),
        [e] => e.to_string(),
        [e, _, ..] => format!("failed to send {} items, sample error: {e}", errors.len()),
    }
}

#[cfg(feature = "pyo3")]
#[derive(Debug, thiserror::Error)]
pub enum SendAllError {
    #[error(transparent)] Clone(#[from] ootr_utils::CloneError),
    #[error(transparent)] Python(#[from] PyErr),
    #[error(transparent)] Write(#[from] async_proto::WriteError),
    #[error("this room is for a different seed")]
    FileHash,
    #[error("{}", display_send_item_errors(.0))]
    Items(Vec<SendItemError>),
}

impl<C: ClientKind> Room<C> {
    async fn write(&mut self, client_id: C::SessionId, msg: &ServerMessage) {
        if let Some(client) = self.clients.get(&client_id) {
            let mut writer = client.writer.lock().await;
            if let Err(e) = writer.write(msg).await {
                eprintln!("error sending message: {e} ({e:?})");
                drop(writer);
                self.remove_client(client_id, EndRoomSession::Disconnect).await;
            }
        }
    }

    async fn write_all(&mut self, msg: &ServerMessage) {
        let mut notified = HashSet::new();
        while let Some((&client_id, client)) = self.clients.iter().find(|&(client_id, _)| !notified.contains(client_id)) {
            let mut writer = client.writer.lock().await;
            if let Err(e) = writer.write(msg).await {
                eprintln!("error sending message: {e} ({e:?})");
                drop(writer);
                self.remove_client(client_id, EndRoomSession::Disconnect).await;
            }
            notified.insert(client_id);
        }
    }

    pub async fn add_client(&mut self, client_id: C::SessionId, writer: Arc<Mutex<C::Writer>>, end_tx: oneshot::Sender<EndRoomSession>) {
        // the client doesn't need to be told that it has connected, so notify everyone *before* adding it
        self.write_all(&ServerMessage::ClientConnected).await;
        self.clients.insert(client_id, Client {
            player: None,
            save_data: None,
            writer, end_tx,
        });
    }

    pub fn has_client(&self, client_id: C::SessionId) -> bool {
        self.clients.contains_key(&client_id)
    }

    #[async_recursion]
    pub async fn remove_client(&mut self, client_id: C::SessionId, to: EndRoomSession) {
        if let Some(client) = self.clients.remove(&client_id) {
            let _ = client.end_tx.send(to);
            let msg = if let Some(Player { world, .. }) = client.player {
                ServerMessage::PlayerDisconnected(world)
            } else {
                ServerMessage::UnregisteredClientDisconnected
            };
            self.write_all(&msg).await;
        }
    }

    pub async fn delete(&mut self) {
        for client_id in self.clients.keys().copied().collect::<Vec<_>>() {
            self.remove_client(client_id, EndRoomSession::ToLobby).await;
        }
        #[cfg(feature = "sqlx")] {
            if let Err(e) = sqlx::query!("DELETE FROM rooms WHERE name = $1", &self.name).execute(&self.db_pool).await {
                eprintln!("failed to delete room from database: {e} ({e:?})");
            }
        }
        #[cfg(feature = "tokio-tungstenite")]
        if let Some((ref tracker_room_name, ref mut sock)) = self.tracker_state {
            let _ = oottracker::websocket::ClientMessage::MwDeleteRoom { room: tracker_room_name.clone() }.write_ws(sock).await;
        }
    }

    /// Moves a player from unloaded (no world assigned) to the given `world`.
    pub async fn load_player(&mut self, client_id: C::SessionId, world: NonZeroU8) -> Result<bool, async_proto::WriteError> {
        if self.clients.iter().any(|(&iter_client_id, iter_client)| iter_client.player.as_ref().map_or(false, |p| p.world == world) && iter_client_id != client_id) {
            return Ok(false)
        }
        let client = self.clients.get_mut(&client_id).expect("no such client");
        #[cfg(feature = "tokio-tungstenite")] let save = client.save_data.clone();
        let prev_player = &mut client.player;
        if let Some(player) = prev_player {
            let prev_world = mem::replace(&mut player.world, world);
            if prev_world == world { return Ok(true) }
            self.write_all(&ServerMessage::ResetPlayerId(prev_world)).await;
        } else {
            *prev_player = Some(Player::new(world));
        }
        self.write_all(&ServerMessage::PlayerId(world)).await;
        let queue = self.player_queues.get(&world).unwrap_or(&self.base_queue).iter().map(|item| item.kind).collect::<Vec<_>>();
        if !queue.is_empty() {
            self.write(client_id, &ServerMessage::ItemQueue(queue)).await;
        }
        #[cfg(feature = "tokio-tungstenite")]
        if let Some(save) = save {
            if let Some((ref tracker_room_name, ref mut sock)) = self.tracker_state {
                oottracker::websocket::ClientMessage::MwResetPlayer { room: tracker_room_name.clone(), world, save }.write_ws(sock).await?;
            }
        }
        Ok(true)
    }

    pub async fn unload_player(&mut self, client_id: C::SessionId) {
        if let Some(prev_player) = self.clients.get_mut(&client_id).expect("no such client").player.take() {
            self.write_all(&ServerMessage::ResetPlayerId(prev_player.world)).await;
        }
    }

    pub async fn set_player_name(&mut self, client_id: C::SessionId, name: Filename) -> bool {
        if let Some(ref mut player) = self.clients.get_mut(&client_id).expect("no such client").player {
            let world = player.world;
            player.name = name;
            drop(player);
            self.write_all(&ServerMessage::PlayerName(world, name)).await;
            true
        } else {
            false
        }
    }

    pub async fn set_file_hash(&mut self, client_id: C::SessionId, hash: [HashIcon; 5]) -> Result<(), SetHashError> {
        if let Some(ref mut player) = self.clients.get_mut(&client_id).expect("no such client").player {
            if self.file_hash.map_or(false, |room_hash| room_hash != hash) {
                return Err(SetHashError::FileHash)
            }
            let world = player.world;
            player.file_hash = Some(hash);
            drop(player);
            self.write_all(&ServerMessage::PlayerFileHash(world, hash)).await;
            Ok(())
        } else {
            Err(SetHashError::NoSourceWorld)
        }
    }

    async fn queue_item_inner(&mut self, source_world: NonZeroU8, key: u32, kind: u16, target_world: NonZeroU8) -> Result<(), async_proto::WriteError> {
        #[cfg(feature = "tokio-tungstenite")]
        if let Some((ref tracker_room_name, ref mut sock)) = self.tracker_state {
            oottracker::websocket::ClientMessage::MwQueueItem {
                room: tracker_room_name.clone(),
                source_world, key, kind, target_world,
            }.write_ws(sock).await?;
        }
        if kind == TRIFORCE_PIECE {
            if !self.base_queue.iter().any(|item| item.source == source_world && item.key == key) {
                self.player_queues.entry(source_world).or_insert_with(|| self.base_queue.clone()); // make sure the sender doesn't get a duplicate of this piece from the base queue
                let item = Item { source: source_world, key, kind };
                self.base_queue.push(item);
                for (&target_world, queue) in &mut self.player_queues {
                    if source_world != target_world {
                        queue.push(item);
                    }
                }
                let msg = ServerMessage::GetItem(kind);
                let player_clients = self.clients.iter()
                    .filter_map(|(&target_client, c)| if c.player.map_or(false, |p| p.world != source_world) { Some(target_client) } else { None })
                    .collect::<Vec<_>>();
                for target_client in player_clients {
                    self.write(target_client, &msg).await;
                }
            }
        } else if source_world == target_world {
            // don't send own item back to sender
        } else {
            if !self.player_queues.get(&target_world).map_or(false, |queue| queue.iter().any(|item| item.source == source_world && item.key == key)) {
                self.player_queues.entry(target_world).or_insert_with(|| self.base_queue.clone()).push(Item { source: source_world, key, kind });
                if let Some((&target_client, _)) = self.clients.iter().find(|(_, c)| c.player.map_or(false, |p| p.world == target_world)) {
                    self.write(target_client, &ServerMessage::GetItem(kind)).await;
                }
            }
        }
        #[cfg(feature = "sqlx")] {
            if let Err(e) = self.save(true).await {
                eprintln!("failed to save room state: {e} ({e:?})");
            }
        }
        Ok(())
    }

    pub async fn queue_item(&mut self, source_client: C::SessionId, key: u32, kind: u16, target_world: NonZeroU8) -> Result<(), QueueItemError> {
        if let Some(source) = self.clients.get(&source_client).expect("no such client").player {
            if let Some(player_hash) = source.file_hash {
                if let Some(room_hash) = self.file_hash {
                    if player_hash != room_hash {
                        return Err(QueueItemError::FileHash)
                    }
                } else {
                    self.file_hash = Some(player_hash);
                }
            }
            self.queue_item_inner(source.world, key, kind, target_world).await?;
            Ok(())
        } else {
            Err(QueueItemError::NoSourceWorld)
        }
    }

    #[cfg(feature = "pyo3")]
    pub async fn send_all(&mut self, source_world: NonZeroU8, spoiler_log: &SpoilerLog) -> Result<(), SendAllError> {
        if self.file_hash.map_or(false, |room_hash| spoiler_log.file_hash != room_hash) {
            return Err(SendAllError::FileHash)
        }
        spoiler_log.version.clone_repo().await?;
        let items_to_queue = Python::with_gil(|py| {
            let py_modules = spoiler_log.version.py_modules(py)?;
            let mut items_to_queue = Vec::default();
            let mut item_errors = Vec::default();
            if let Some(world_locations) = spoiler_log.locations.get(usize::from(source_world.get() - 1)) {
                for (loc, ootr_utils::spoiler::Item { player, item, model: _ }) in world_locations {
                    if *player != source_world {
                        if let Some(key) = py_modules.override_key(loc, item)? {
                            if let Some(kind) = py_modules.item_kind(item)? {
                                items_to_queue.push((source_world, key, kind, *player));
                            } else {
                                item_errors.push(SendItemError::Kind(item.clone()));
                            }
                        } else {
                            item_errors.push(SendItemError::Key(loc.clone()));
                        }
                    }
                }
                Ok(items_to_queue)
            } else {
                Err(SendAllError::Items(item_errors))
            }
        })?;
        for (source_world, key, kind, target_world) in items_to_queue {
            self.queue_item_inner(source_world, key, kind, target_world).await?;
        }
        Ok(())
    }

    pub async fn set_save_data(&mut self, client_id: C::SessionId, save: oottracker::Save) -> Result<(), async_proto::WriteError> {
        let client = self.clients.get_mut(&client_id).expect("no such client");
        client.save_data = Some(save.clone());
        #[cfg(feature = "tokio-tungstenite")]
        if let Some(Player { world, .. }) = client.player {
            if let Some((ref tracker_room_name, ref mut sock)) = self.tracker_state {
                oottracker::websocket::ClientMessage::MwResetPlayer { room: tracker_room_name.clone(), world, save }.write_ws(sock).await?;
            }
        }
        Ok(())
    }

    #[cfg(feature = "tokio-tungstenite")]
    pub async fn init_tracker(&mut self, tracker_room_name: String, world_count: NonZeroU8) -> Result<(), async_proto::WriteError> {
        let mut worlds = (1..=world_count.get())
            .map(|player_id| (
                None,
                self.player_queues.get(&NonZeroU8::new(player_id).expect("range starts at 1")).unwrap_or(&self.base_queue).clone(),
            ))
            .collect::<Vec<_>>();
        for client in self.clients.values() {
            if let (Some(player), Some(save_data)) = (client.player, client.save_data) {
                worlds[usize::from(player.world.get() - 1)].0 = Some(save_data);
            }
        }
        let mut sock = tokio_tungstenite::connect_async("wss://oottracker.fenhl.net/websocket").await?.0;
        oottracker::websocket::ClientMessage::MwCreateRoom { room: tracker_room_name.clone(), worlds }.write_ws(&mut sock).await?;
        self.tracker_state = Some((tracker_room_name, sock));
        Ok(())
    }

    pub fn autodelete_at(&self) -> DateTime<Utc> {
        self.last_saved + chrono::Duration::from_std(self.autodelete_delta).expect("autodelete delta too long")
    }

    #[cfg(feature = "sqlx")]
    pub async fn save(&mut self, update_last_saved: bool) -> sqlx::Result<()> {
        let mut base_queue = Vec::default();
        self.base_queue.write_sync(&mut base_queue).expect("failed to write base queue to buffer");
        let mut player_queues = Vec::default();
        self.player_queues.write_sync(&mut player_queues).expect("failed to write player queues to buffer");
        if update_last_saved {
            self.last_saved = Utc::now();
            let _ = self.autodelete_tx.send((self.name.clone(), self.autodelete_at()));
        }
        sqlx::query!("INSERT INTO rooms (
            name,
            password_hash,
            password_salt,
            base_queue,
            player_queues,
            last_saved,
            autodelete_delta
        ) VALUES ($1, $2, $3, $4, $5, $6, $7) ON CONFLICT (name) DO UPDATE SET
            password_hash = EXCLUDED.password_hash,
            password_salt = EXCLUDED.password_salt,
            base_queue = EXCLUDED.base_queue,
            player_queues = EXCLUDED.player_queues,
            last_saved = EXCLUDED.last_saved,
            autodelete_delta = EXCLUDED.autodelete_delta
        ", &self.name, &self.password_hash, &self.password_salt, base_queue, player_queues, self.last_saved, self.autodelete_delta as _).execute(&self.db_pool).await?;
        Ok(())
    }

    pub async fn set_autodelete_delta(&mut self, new_delta: Duration) {
        self.autodelete_delta = new_delta;
        #[cfg(feature = "sqlx")] {
            // saving also notifies the room deletion waiter
            if let Err(e) = self.save(true).await {
                eprintln!("failed to save room state: {e} ({e:?})");
            }
        }
        self.write_all(&ServerMessage::AutoDeleteDelta(new_delta)).await;
    }
}

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
    /// The client has the wrong seed loaded.
    #[error("wrong file hash")]
    WrongFileHash,
    /// The client attempted to create a room with a duplicate name.
    #[error("a room with this name already exists")]
    RoomExists,
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
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error(transparent)] Read(#[from] async_proto::ReadError),
    #[error(transparent)] Write(#[from] async_proto::WriteError),
    #[error("protocol version mismatch: server is version {0} but we're version {}", proto_version())]
    VersionMismatch(u8),
}

impl IsNetworkError for ClientError {
    fn is_network_error(&self) -> bool {
        match self {
            Self::Read(e) => e.is_network_error(),
            Self::Write(e) => e.is_network_error(),
            Self::VersionMismatch(_) => false,
        }
    }
}

pub async fn handshake(tcp_stream: &mut TcpStream) -> Result<(), ClientError> {
    proto_version().write(tcp_stream).await?;
    let server_version = u8::read(tcp_stream).await?;
    if server_version != proto_version() { return Err(ClientError::VersionMismatch(server_version)) }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum SessionStateError<E> {
    #[error(transparent)] Connection(#[from] E),
    #[error("server error #{0}")]
    Future(u8),
    #[error("received an unexpected message from the server given the current connection state")]
    Mismatch,
    #[error("server error: {0}")]
    Server(String),
}

#[derive(Debug, Clone, Copy)]
pub enum RoomView {
    Normal,
    ConfirmDeletion,
    Options,
}

#[derive(Debug)]
pub enum SessionState<E> {
    Error {
        e: SessionStateError<E>,
        auto_retry: bool,
    },
    Init,
    InitAutoRejoin {
        room_name: String,
        room_password: String,
    },
    Lobby {
        logged_in_as_admin: bool,
        rooms: BTreeSet<String>,
        create_new_room: bool,
        existing_room_selection: Option<String>,
        new_room_name: String,
        password: String,
        wrong_password: bool,
    },
    Room {
        room_name: String,
        room_password: String,
        players: Vec<Player>,
        num_unassigned_clients: u8,
        item_queue: Vec<u16>,
        autodelete_delta: Duration,
        view: RoomView,
        wrong_file_hash: bool,
    },
    Closed,
}

impl<E> SessionState<E> {
    pub fn apply(&mut self, msg: ServerMessage) {
        match msg {
            ServerMessage::Ping => {}
            ServerMessage::StructuredError(ServerError::WrongPassword) => if let SessionState::Lobby { password, wrong_password, .. } = self {
                *wrong_password = true;
                password.clear();
            } else {
                *self = Self::Error {
                    e: SessionStateError::Mismatch,
                    auto_retry: false,
                };
            },
            ServerMessage::StructuredError(ServerError::WrongFileHash) => if let SessionState::Room { wrong_file_hash, .. } = self {
                *wrong_file_hash = true;
            } else {
                *self = Self::Error {
                    e: SessionStateError::Mismatch,
                    auto_retry: false,
                };
            },
            ServerMessage::StructuredError(ServerError::RoomExists) => if let SessionState::Lobby { create_new_room: ref mut create_new_room @ true, .. } = self {
                *create_new_room = false;
            } else {
                *self = Self::Error {
                    e: SessionStateError::Mismatch,
                    auto_retry: false,
                };
            },
            ServerMessage::StructuredError(ServerError::Future(discrim)) => if !matches!(self, SessionState::Error { .. }) {
                *self = SessionState::Error {
                    e: SessionStateError::Future(discrim),
                    auto_retry: false,
                };
            },
            ServerMessage::OtherError(e) => if !matches!(self, SessionState::Error { .. }) {
                *self = SessionState::Error {
                    e: SessionStateError::Server(e),
                    auto_retry: false,
                };
            },
            ServerMessage::EnterLobby { rooms } => *self = if let SessionState::InitAutoRejoin { room_name, room_password } = self {
                let room_still_exists = rooms.contains(room_name);
                SessionState::Lobby {
                    logged_in_as_admin: false,
                    create_new_room: !room_still_exists,
                    existing_room_selection: room_still_exists.then(|| room_name.clone()),
                    new_room_name: room_name.clone(),
                    password: room_password.clone(),
                    wrong_password: false,
                    rooms,
                }
            } else {
                SessionState::Lobby {
                    logged_in_as_admin: false,
                    create_new_room: rooms.is_empty(),
                    existing_room_selection: None,
                    new_room_name: String::default(),
                    password: String::default(),
                    wrong_password: false,
                    rooms,
                }
            },
            ServerMessage::NewRoom(name) => if let SessionState::Lobby { rooms, .. } = self {
                rooms.insert(name);
            } else {
                *self = Self::Error {
                    e: SessionStateError::Mismatch,
                    auto_retry: false,
                };
            },
            ServerMessage::DeleteRoom(name) => if let SessionState::Lobby { rooms, .. } = self {
                rooms.remove(&name);
            } else {
                *self = Self::Error {
                    e: SessionStateError::Mismatch,
                    auto_retry: false,
                };
            },
            ServerMessage::EnterRoom { players, num_unassigned_clients, autodelete_delta } => {
                let (room_name, room_password) = match self {
                    SessionState::Lobby { create_new_room: false, existing_room_selection, password, .. } => (existing_room_selection.clone().unwrap_or_default(), password.clone()),
                    SessionState::Lobby { create_new_room: true, new_room_name, password, .. } => (new_room_name.clone(), password.clone()),
                    _ => <_>::default(),
                };
                *self = SessionState::Room {
                    item_queue: Vec::default(),
                    view: RoomView::Normal,
                    wrong_file_hash: false,
                    room_name, room_password, players, num_unassigned_clients, autodelete_delta,
                };
            }
            ServerMessage::PlayerId(world) => if let SessionState::Room { players, num_unassigned_clients, .. } = self {
                if let Err(idx) = players.binary_search_by_key(&world, |p| p.world) {
                    players.insert(idx, Player::new(world));
                    *num_unassigned_clients -= 1;
                }
            } else {
                *self = Self::Error {
                    e: SessionStateError::Mismatch,
                    auto_retry: false,
                };
            },
            ServerMessage::ResetPlayerId(world) => if let SessionState::Room { players, num_unassigned_clients, .. } = self {
                if let Ok(idx) = players.binary_search_by_key(&world, |p| p.world) {
                    players.remove(idx);
                    *num_unassigned_clients += 1;
                }
            } else {
                *self = Self::Error {
                    e: SessionStateError::Mismatch,
                    auto_retry: false,
                };
            },
            ServerMessage::ClientConnected => if let SessionState::Room { num_unassigned_clients, .. } = self {
                *num_unassigned_clients += 1;
            } else {
                *self = Self::Error {
                    e: SessionStateError::Mismatch,
                    auto_retry: false,
                };
            },
            ServerMessage::PlayerDisconnected(world) => if let SessionState::Room { players, .. } = self {
                if let Ok(idx) = players.binary_search_by_key(&world, |p| p.world) {
                    players.remove(idx);
                }
            } else {
                *self = Self::Error {
                    e: SessionStateError::Mismatch,
                    auto_retry: false,
                };
            },
            ServerMessage::UnregisteredClientDisconnected => if let SessionState::Room { num_unassigned_clients, .. } = self {
                *num_unassigned_clients -= 1;
            } else {
                *self = Self::Error {
                    e: SessionStateError::Mismatch,
                    auto_retry: false,
                };
            },
            ServerMessage::PlayerName(world, name) => if let SessionState::Room { players, .. } = self {
                if let Ok(idx) = players.binary_search_by_key(&world, |p| p.world) {
                    players[idx].name = name;
                }
            } else {
                *self = Self::Error {
                    e: SessionStateError::Mismatch,
                    auto_retry: false,
                };
            },
            ServerMessage::ItemQueue(queue) => if let SessionState::Room { item_queue, .. } = self {
                *item_queue = queue.clone();
            } else {
                *self = Self::Error {
                    e: SessionStateError::Mismatch,
                    auto_retry: false,
                };
            },
            ServerMessage::GetItem(item) => if let SessionState::Room { item_queue, .. } = self {
                item_queue.push(item);
            } else {
                *self = Self::Error {
                    e: SessionStateError::Mismatch,
                    auto_retry: false,
                };
            },
            ServerMessage::AdminLoginSuccess { .. } => if let SessionState::Lobby { logged_in_as_admin, .. } = self {
                *logged_in_as_admin = true;
            } else {
                *self = Self::Error {
                    e: SessionStateError::Mismatch,
                    auto_retry: false,
                };
            },
            ServerMessage::Goodbye => if !matches!(self, SessionState::Error { .. }) {
                *self = SessionState::Closed;
            },
            ServerMessage::PlayerFileHash(world, hash) => if let SessionState::Room { players, .. } = self {
                if let Ok(idx) = players.binary_search_by_key(&world, |p| p.world) {
                    players[idx].file_hash = Some(hash);
                }
            } else {
                *self = Self::Error {
                    e: SessionStateError::Mismatch,
                    auto_retry: false,
                };
            },
            ServerMessage::AutoDeleteDelta(new_delta) => if let SessionState::Room { autodelete_delta, .. } = self {
                *autodelete_delta = new_delta;
            } else {
                *self = Self::Error {
                    e: SessionStateError::Mismatch,
                    auto_retry: false,
                };
            },
            ServerMessage::RoomsEmpty => {}
        }
    }
}

pub fn format_room_state(players: &[Player], num_unassigned_clients: u8, my_world: Option<NonZeroU8>) -> (Vec<(NonZeroU8, String)>, String) {
    match (players.len(), num_unassigned_clients) {
        (0, 0) => (Vec::default(), format!("this room is empty")), // for admin view
        (0, unassigned) => (Vec::default(), format!("{unassigned} client{} with no world", if unassigned == 1 { "" } else { "s" })),
        (_, unassigned) => {
            (players.iter()
                .map(|player| (player.world, if player.name == Filename::default() {
                    if my_world == Some(player.world) {
                        format!("{}. [create save file 1 to set name]", player.world)
                    } else {
                        format!("{}. [unnamed]", player.world)
                    }
                } else {
                    format!("{}. {}", player.world, player.name)
                }))
                .collect(),
            if unassigned > 0 {
                format!("…and {unassigned} client{} with no world", if unassigned == 1 { "" } else { "s" })
            } else {
                String::default()
            })
        }
    }
}
