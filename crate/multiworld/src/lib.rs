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
        mem,
        net::{
            Ipv4Addr,
            Ipv6Addr,
        },
        num::NonZeroU8,
        sync::Arc,
        time::Instant,
    },
    async_proto::Protocol,
    async_recursion::async_recursion,
    chrono::prelude::*,
    semver::Version,
    tokio::{
        io,
        net::{
            TcpStream,
            tcp::OwnedWriteHalf,
        },
        sync::Mutex,
    },
};
#[cfg(unix)] use std::os::unix::io::AsRawFd;
#[cfg(windows)] use std::os::windows::io::AsRawSocket;
#[cfg(feature = "sqlx")] use {
    std::time::Duration,
    sqlx::PgPool,
};

pub mod github;
#[cfg(feature = "style")] pub mod style;

pub const ADDRESS_V4: Ipv4Addr = Ipv4Addr::new(37, 252, 122, 84);
pub const ADDRESS_V6: Ipv6Addr = Ipv6Addr::new(0x2a02, 0x2770, 0x8, 0, 0x21a, 0x4aff, 0xfee1, 0xf281);
pub const PORT: u16 = 24809;

pub fn version() -> Version { Version::parse(env!("CARGO_PKG_VERSION")).expect("failed to parse package version") }
pub fn proto_version() -> u8 { version().major.try_into().expect("version number does not fit into u8") }

const TRIFORCE_PIECE: u16 = 0xca;

#[cfg(unix)] pub type SocketId = std::os::unix::io::RawFd;
#[cfg(windows)] pub type SocketId = std::os::windows::io::RawSocket;

#[cfg(unix)] pub fn socket_id<T: AsRawFd>(socket: &T) -> SocketId { socket.as_raw_fd() }
#[cfg(windows)] pub fn socket_id<T: AsRawSocket>(socket: &T) -> SocketId { socket.as_raw_socket() }

pub trait IsNetworkError {
    fn is_network_error(&self) -> bool;
}

impl IsNetworkError for io::Error {
    fn is_network_error(&self) -> bool {
        //TODO io::ErrorKind::NetworkUnreachable should also be considered here, as it can occur during a server reboot, but it is currently unstable, making it impossible to match against. See https://github.com/rust-lang/rust/issues/86442
        matches!(self.kind(), io::ErrorKind::ConnectionAborted | io::ErrorKind::ConnectionRefused | io::ErrorKind::ConnectionReset | io::ErrorKind::TimedOut | io::ErrorKind::UnexpectedEof)
    }
}

impl IsNetworkError for async_proto::ReadError {
    fn is_network_error(&self) -> bool {
        match self {
            Self::EndOfStream => true,
            Self::Io(e) => e.is_network_error(),
            _ => false,
        }
    }
}

impl IsNetworkError for async_proto::WriteError {
    fn is_network_error(&self) -> bool {
        match self {
            Self::Io(e) => e.is_network_error(),
            _ => false,
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
}

impl Player {
    pub fn new(world: NonZeroU8) -> Self {
        Self {
            world,
            name: Filename::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Protocol)]
pub struct Item {
    pub source: NonZeroU8,
    pub key: u32,
    pub kind: u16,
}

#[derive(Debug)]
pub struct Room {
    pub name: String,
    pub password: String,
    pub clients: HashMap<SocketId, (Option<Player>, Arc<Mutex<OwnedWriteHalf>>)>,
    pub base_queue: Vec<Item>,
    pub player_queues: HashMap<NonZeroU8, Vec<Item>>,
    pub last_saved: Instant,
    #[cfg(feature = "sqlx")]
    pub db_pool: PgPool,
}

impl Room {
    async fn write(&mut self, client_id: SocketId, msg: &ServerMessage) {
        if let Some((_, writer)) = self.clients.get(&client_id) {
            let mut writer = writer.lock().await;
            if let Err(e) = msg.write(&mut *writer).await {
                eprintln!("{} error sending message: {:?}", Utc::now().format("%Y-%m-%d %H:%M:%S"), e);
                drop(writer);
                self.remove_client(client_id).await;
            }
        }
    }

    async fn write_all(&mut self, msg: &ServerMessage) {
        let mut notified = HashSet::new();
        while let Some((&client_id, (_, writer))) = self.clients.iter().find(|&(client_id, _)| !notified.contains(client_id)) {
            let mut writer = writer.lock().await;
            if let Err(e) = msg.write(&mut *writer).await {
                eprintln!("{} error sending message: {:?}", Utc::now().format("%Y-%m-%d %H:%M:%S"), e);
                drop(writer);
                self.remove_client(client_id).await;
            }
            notified.insert(client_id);
        }
    }

    pub async fn add_client(&mut self, client_id: SocketId, writer: Arc<Mutex<OwnedWriteHalf>>) {
        // the client doesn't need to be told that it has connected, so notify everyone *before* adding it
        self.write_all(&ServerMessage::ClientConnected).await;
        self.clients.insert(client_id, (None, writer));
    }

    pub fn has_client(&self, client_id: SocketId) -> bool {
        self.clients.contains_key(&client_id)
    }

    #[async_recursion]
    pub async fn remove_client(&mut self, client_id: SocketId) {
        if let Some((player, writer)) = self.clients.remove(&client_id) {
            let _ = ServerMessage::Goodbye.write(&mut *writer.lock().await).await;
            let msg = if let Some(Player { world, .. }) = player {
                ServerMessage::PlayerDisconnected(world)
            } else {
                ServerMessage::UnregisteredClientDisconnected
            };
            self.write_all(&msg).await;
        }
    }

    /// Moves a player from unloaded (no world assigned) to the given `world`.
    pub async fn load_player(&mut self, client_id: SocketId, world: NonZeroU8) -> bool {
        if self.clients.iter().any(|(&iter_client_id, (iter_player, _))| iter_player.as_ref().map_or(false, |p| p.world == world) && iter_client_id != client_id) {
            return false
        }
        let prev_player = &mut self.clients.get_mut(&client_id).expect("no such client").0;
        if let Some(player) = prev_player {
            let prev_world = mem::replace(&mut player.world, world);
            if prev_world == world { return true }
            self.write_all(&ServerMessage::ResetPlayerId(prev_world)).await;
        } else {
            *prev_player = Some(Player::new(world));
        }
        self.write_all(&ServerMessage::PlayerId(world)).await;
        let queue = self.player_queues.get(&world).unwrap_or(&self.base_queue).iter().map(|item| item.kind).collect::<Vec<_>>();
        if !queue.is_empty() {
            self.write(client_id, &ServerMessage::ItemQueue(queue)).await;
        }
        true
    }

    pub async fn unload_player(&mut self, client_id: SocketId) {
        if let Some(prev_player) = self.clients.get_mut(&client_id).expect("no such client").0.take() {
            self.write_all(&ServerMessage::ResetPlayerId(prev_player.world)).await;
        }
    }

    pub async fn set_player_name(&mut self, client_id: SocketId, name: Filename) -> bool {
        if let Some(ref mut player) = self.clients.get_mut(&client_id).expect("no such client").0 {
            let world = player.world;
            player.name = name;
            drop(player);
            self.write_all(&ServerMessage::PlayerName(world, name)).await;
            true
        } else {
            false
        }
    }

    pub async fn queue_item(&mut self, source_client: SocketId, key: u32, kind: u16, target_world: NonZeroU8) -> bool {
        if let Some(source) = self.clients.get(&source_client).expect("no such client").0.map(|source_player| source_player.world) {
            if kind == TRIFORCE_PIECE {
                if !self.base_queue.iter().any(|item| item.source == source && item.key == key) {
                    let item = Item { source, key, kind };
                    self.base_queue.push(item);
                    for queue in self.player_queues.values_mut() {
                        queue.push(item);
                    }
                    let msg = ServerMessage::GetItem(kind);
                    let player_clients = self.clients.iter()
                        .filter_map(|(&target_client, (p, _))| if p.map_or(false, |p| p.world != source) { Some(target_client) } else { None })
                        .collect::<Vec<_>>();
                    for target_client in player_clients {
                        self.write(target_client, &msg).await;
                    }
                }
            } else {
                if !self.player_queues.get(&target_world).map_or(false, |queue| queue.iter().any(|item| item.source == source && item.key == key)) {
                    self.player_queues.entry(target_world).or_insert_with(|| self.base_queue.clone()).push(Item { source, key, kind });
                    if let Some((&target_client, _)) = self.clients.iter().find(|(_, (p, _))| p.map_or(false, |p| p.world == target_world)) {
                        self.write(target_client, &ServerMessage::GetItem(kind)).await;
                    }
                }
            }
            #[cfg(feature = "sqlx")] {
                if let Err(e) = self.save().await {
                    eprintln!("failed to save room state: {e} ({e:?})");
                }
            }
            true
        } else {
            false
        }
    }

    #[cfg(feature = "sqlx")]
    async fn save(&mut self) -> sqlx::Result<()> {
        if self.last_saved.elapsed() >= Duration::from_secs(60) {
            self.force_save().await?;
        }
        Ok(())
    }

    #[cfg(feature = "sqlx")]
    pub async fn force_save(&mut self) -> sqlx::Result<()> {
        let mut base_queue = Vec::default();
        self.base_queue.write_sync(&mut base_queue).expect("failed to write base queue to buffer");
        let mut player_queues = Vec::default();
        self.player_queues.write_sync(&mut player_queues).expect("failed to write player queues to buffer");
        sqlx::query!("INSERT INTO rooms (name, password, base_queue, player_queues) VALUES ($1, $2, $3, $4) ON CONFLICT (name) DO UPDATE SET password = EXCLUDED.password, base_queue = EXCLUDED.base_queue, player_queues = EXCLUDED.player_queues", &self.name, &self.password, base_queue, player_queues).execute(&self.db_pool).await?;
        self.last_saved = Instant::now();
        Ok(())
    }
}

#[derive(Protocol)]
pub enum LobbyClientMessage {
    /// Tells the server we're still here. Should be sent every 30 seconds; the server will consider the connection lost if no message is received for 60 seconds.
    Ping,
    JoinRoom {
        name: String,
        password: String,
    },
    CreateRoom {
        name: String,
        password: String,
    },
    Login {
        id: u64,
        api_key: [u8; 32],
    },
}

#[derive(Protocol)]
pub enum AdminClientMessage {
    /// Tells the server we're still here. Should be sent every 30 seconds; the server will consider the connection lost if no message is received for 60 seconds.
    Ping,
    /// Stops the server.
    Stop,
}

#[derive(Protocol)]
pub enum RoomClientMessage {
    /// Tells the server we're still here. Should be sent every 30 seconds; the server will consider the connection lost if no message is received for 60 seconds.
    Ping,
    /// Claims a world.
    PlayerId(NonZeroU8),
    /// Unloads the previously claimed world.
    ResetPlayerId,
    /// Player names are encoded in the NTSC charset, with trailing spaces (`0xdf`).
    PlayerName(Filename),
    SendItem {
        key: u32,
        kind: u16,
        target_world: NonZeroU8,
    },
    KickPlayer(NonZeroU8),
}

macro_rules! server_errors {
    ($(#[$attr:meta] $variant:ident),* $(,)?) => {
        /// New unit variants on this enum don't cause a major version bump, since the client interprets them as instances of the `Future` variant.
        #[derive(Debug, Clone, Copy, Protocol)]
        #[async_proto(via = u8, clone)]
        pub enum ServerError {
            /// The server sent a `ServerError` that the client doesn't know about yet.
            Future(u8),
            $(#[$attr] $variant,)*
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
    WrongPassword,
}

#[derive(Debug, Clone, Protocol)]
pub enum ServerMessage {
    /// Tells the client we're still here. Sent every 30 seconds; clients should consider the connection lost if no message is received for 60 seconds.
    Ping,
    /// An error that the client might be able to recover from has occurred.
    StructuredError(ServerError),
    /// A fatal error has occurred. Contains a human-readable error message.
    OtherError(String),
    /// You have just connected or left a room and are now sending [`LobbyClientMessage`]s.
    EnterLobby {
        rooms: BTreeSet<String>,
    },
    /// A new room has been created.
    NewRoom(String),
    /// You have created or joined a room and are now sending [`RoomClientMessage`]s.
    EnterRoom {
        players: Vec<Player>,
        num_unassigned_clients: u8,
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
    /// You have logged in as an admin and are now sending [`AdminClientMessage`]s.
    AdminLoginSuccess {
        active_connections: BTreeMap<String, (Vec<Player>, u8)>,
    },
    /// The client will now be disconnected.
    Goodbye,
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

pub fn handshake_sync(tcp_stream: &mut std::net::TcpStream) -> Result<(), ClientError> {
    proto_version().write_sync(tcp_stream)?;
    let server_version = u8::read_sync(tcp_stream)?;
    if server_version != proto_version() { return Err(ClientError::VersionMismatch(server_version)) }
    Ok(())
}

pub fn format_room_state(players: &[Player], num_unassigned_clients: u8, my_world: Option<NonZeroU8>) -> (Vec<String>, String) {
    match (players.len(), num_unassigned_clients) {
        (0, 0) => (Vec::default(), format!("this room is empty")), // for admin view
        (0, unassigned) => (Vec::default(), format!("{unassigned} client{} with no world", if unassigned == 1 { "" } else { "s" })),
        (_, unassigned) => {
            (players.iter()
                .map(|player| if player.name == Filename::default() {
                    if my_world == Some(player.world) {
                        format!("{}. [create save file 1 to set name]", player.world)
                    } else {
                        format!("{}. [unnamed]", player.world)
                    }
                } else {
                    format!("{}. {}", player.world, player.name)
                })
                .collect(),
            if unassigned > 0 {
                format!("…and {unassigned} client{} with no world", if unassigned == 1 { "" } else { "s" })
            } else {
                String::default()
            })
        }
    }
}
