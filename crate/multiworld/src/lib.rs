use {
    std::{
        collections::{
            BTreeMap,
            HashMap,
            HashSet,
        },
        fmt,
        hash::Hash,
        mem,
        num::NonZeroU8,
        str::FromStr,
        sync::Arc,
        time::Duration,
    },
    async_proto::Protocol,
    async_trait::async_trait,
    bitflags::bitflags,
    chrono::{
        TimeDelta,
        prelude::*,
    },
    derivative::Derivative,
    itertools::Itertools as _,
    log_lock::{
        Mutex,
        lock,
    },
    ootr::model::{
        DungeonReward,
        DungeonRewardLocation,
        MainDungeon,
    },
    ootr_utils::spoiler::{
        HashIcon,
        SpoilerLog,
    },
    oottracker::websocket::MwItem as Item,
    semver::Version,
    serde::{
        Deserialize,
        Serialize,
    },
    tokio::{
        net::{
            TcpStream,
            tcp::{
                OwnedReadHalf,
                OwnedWriteHalf,
            },
        },
        sync::{
            broadcast,
            oneshot,
        },
    },
    wheel::traits::IsNetworkError,
    crate::ws::{
        ServerError,
        latest,
        unversioned,
    },
};
#[cfg(unix)] use std::os::unix::io::AsRawFd;
#[cfg(windows)] use std::os::windows::io::AsRawSocket;
#[cfg(target_os = "linux")] use {
    std::{
        os::unix::fs::PermissionsExt as _,
        path::PathBuf,
        pin::Pin,
    },
    futures::{
        future::Future,
        stream::TryStreamExt as _,
    },
    wheel::fs,
};
#[cfg(feature = "sqlx")] use sqlx::PgPool;

pub mod config;
pub mod frontend;
pub mod github;
pub mod ws;

pub const DEFAULT_TCP_PORT: u16 = 24809; //TODO use for LAN support (https://github.com/midoshouse/ootr-multiworld/issues/3)

pub const CREDENTIAL_LEN: usize = ring::digest::SHA512_OUTPUT_LEN;

pub fn version() -> Version { Version::parse(env!("CARGO_PKG_VERSION")).expect("failed to parse package version") }
pub fn proto_version() -> u8 { version().major.try_into().expect("version number does not fit into u8") }

/// The server (ootrmwd) will be stopped with advance warning when a new version of the server is deployed (multiworld-release).
/// This advance warning can be skipped if there are no active rooms.
/// The server reports the current state to the release script using this message type.
#[derive(Protocol)]
pub enum WaitUntilInactiveMessage {
    Error,
    ActiveRooms(HashMap<String, (DateTime<Utc>, u64)>),
    Inactive,
    Deadline(DateTime<Utc>),
}

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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RoomFormatter {
    pub password_required: bool,
    pub name: String,
    pub id: u64,
    pub active: bool,
}

impl fmt::Display for RoomFormatter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.name.fmt(f)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Protocol)]
pub enum HintArea {
    Root,
    HyruleField,
    LonLonRanch,
    Market,
    TempleOfTime,
    HyruleCastle,
    OutsideGanonsCastle,
    InsideGanonsCastle,
    KokiriForest,
    DekuTree,
    LostWoods,
    SacredForestMeadow,
    ForestTemple,
    DeathMountainTrail,
    DodongosCavern,
    GoronCity,
    DeathMountainCrater,
    FireTemple,
    ZoraRiver,
    ZorasDomain,
    ZorasFountain,
    JabuJabusBelly,
    IceCavern,
    LakeHylia,
    WaterTemple,
    KakarikoVillage,
    BottomOfTheWell,
    Graveyard,
    ShadowTemple,
    GerudoValley,
    GerudoFortress,
    ThievesHideout,
    GerudoTrainingGround,
    HauntedWasteland,
    DesertColossus,
    SpiritTemple,
}

impl TryFrom<HintArea> for DungeonRewardLocation {
    type Error = ();

    fn try_from(area: HintArea) -> Result<Self, ()> {
        match area {
            HintArea::Root => Ok(Self::LinksPocket),
            HintArea::HyruleField => Err(()),
            HintArea::LonLonRanch => Err(()),
            HintArea::Market => Err(()),
            HintArea::TempleOfTime => Err(()),
            HintArea::HyruleCastle => Err(()),
            HintArea::OutsideGanonsCastle => Err(()),
            HintArea::InsideGanonsCastle => Err(()),
            HintArea::KokiriForest => Err(()),
            HintArea::DekuTree => Ok(Self::Dungeon(MainDungeon::DekuTree)),
            HintArea::LostWoods => Err(()),
            HintArea::SacredForestMeadow => Err(()),
            HintArea::ForestTemple => Ok(Self::Dungeon(MainDungeon::ForestTemple)),
            HintArea::DeathMountainTrail => Err(()),
            HintArea::DodongosCavern => Ok(Self::Dungeon(MainDungeon::DodongosCavern)),
            HintArea::GoronCity => Err(()),
            HintArea::DeathMountainCrater => Err(()),
            HintArea::FireTemple => Ok(Self::Dungeon(MainDungeon::FireTemple)),
            HintArea::ZoraRiver => Err(()),
            HintArea::ZorasDomain => Err(()),
            HintArea::ZorasFountain => Err(()),
            HintArea::JabuJabusBelly => Ok(Self::Dungeon(MainDungeon::JabuJabu)),
            HintArea::IceCavern => Err(()),
            HintArea::LakeHylia => Err(()),
            HintArea::WaterTemple => Ok(Self::Dungeon(MainDungeon::WaterTemple)),
            HintArea::KakarikoVillage => Err(()),
            HintArea::BottomOfTheWell => Err(()),
            HintArea::Graveyard => Err(()),
            HintArea::ShadowTemple => Ok(Self::Dungeon(MainDungeon::ShadowTemple)),
            HintArea::GerudoValley => Err(()),
            HintArea::GerudoFortress => Err(()),
            HintArea::ThievesHideout => Err(()),
            HintArea::GerudoTrainingGround => Err(()),
            HintArea::HauntedWasteland => Err(()),
            HintArea::DesertColossus => Err(()),
            HintArea::SpiritTemple => Ok(Self::Dungeon(MainDungeon::SpiritTemple)),
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
                Self([0xba, 0xd0, 0xc5, 0xdd, 0xd6, hundreds, tens, ones]) // PlayrNNN
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

#[derive(Debug, thiserror::Error)]
pub enum FilenameParseError {
    #[error("the character {0:?} is not allowed in OoT filenames")]
    Char(char),
    #[error("OoT filename too long (got {0} characters, maximum is 8)")]
    TooLong(usize),
}

impl FromStr for Filename {
    type Err = FilenameParseError;

    fn from_str(s: &str) -> Result<Self, FilenameParseError> {
        let mut buf = s.chars().map(|c| if c == '�' {
            Err(FilenameParseError::Char('�'))
        } else {
            Self::ENCODING.into_iter()
                .position(|iter_char| iter_char == c)
                .map(|pos| u8::try_from(pos).expect("more than 256 characters in Filename::ENCODING"))
                .ok_or_else(|| FilenameParseError::Char(c))
        }).try_collect::<_, Vec<_>, _>()?;
        if buf.len() < 8 {
            buf.resize(8, 0xdf);
        }
        buf.try_into().map(Self).map_err(|buf| FilenameParseError::TooLong(buf.len()))
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
pub trait ClientReader: Unpin + Send + Sized {
    async fn read_owned(self) -> Result<(Self, unversioned::ClientMessage), async_proto::ReadError>;
}

#[async_trait]
pub trait ClientWriter: Unpin + Send {
    async fn write(&mut self, msg: unversioned::ServerMessage) -> Result<(), async_proto::WriteError>;
}

pub trait ClientKind {
    type SessionId: fmt::Debug + Copy + Eq + Hash + Send + Sync;
    type Reader: ClientReader + 'static;
    type Writer: ClientWriter + 'static;
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
    async fn read_owned(self) -> Result<(Self, unversioned::ClientMessage), async_proto::ReadError> {
        let (reader, msg) = latest::ClientMessage::read_owned(self).await?;
        Ok((reader, msg.try_into()?))
    }
}

#[async_trait]
impl ClientWriter for OwnedWriteHalf {
    async fn write(&mut self, msg: unversioned::ServerMessage) -> Result<(), async_proto::WriteError> {
        if let Some(msg) = Option::<latest::ServerMessage>::from(msg) {
            msg.write(self).await?;
        }
        Ok(())
    }
}

pub struct Client<C: ClientKind> {
    pub writer: Arc<Mutex<C::Writer>>,
    pub end_tx: oneshot::Sender<EndRoomSession>,
    pub player: Option<Player>,
    pub pending_world: Option<NonZeroU8>,
    pub pending_name: Option<Filename>,
    pub pending_hash: Option<[HashIcon; 5]>,
    pub pending_items: Vec<(u64, u16, NonZeroU8)>,
    pub tracker_state: oottracker::ModelState,
    pub adjusted_save: oottracker::Save,
}

impl<C: ClientKind> fmt::Debug for Client<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self { writer: _, end_tx, player, pending_world, pending_name, pending_hash, pending_items, tracker_state, adjusted_save } = self;
        f.debug_struct("Client")
            .field("writer", &format_args!("_"))
            .field("end_tx", end_tx)
            .field("player", player)
            .field("pending_world", pending_world)
            .field("pending_name", pending_name)
            .field("pending_hash", pending_hash)
            .field("pending_items", pending_items)
            .field("tracker_state", tracker_state)
            .field("adjusted_save", adjusted_save)
            .finish()
    }
}

#[derive(Clone)]
pub enum RoomAuth {
    Password {
        hash: [u8; CREDENTIAL_LEN],
        salt: [u8; CREDENTIAL_LEN],
    },
    Invitational(Vec<u64>),
}

impl RoomAuth {
    pub fn availability(&self, logged_in_as_admin: bool, midos_house_user_id: Option<u64>) -> RoomAvailability {
        if logged_in_as_admin {
            RoomAvailability::Open
        } else {
            match self {
                Self::Password { .. } => RoomAvailability::PasswordRequired,
                Self::Invitational(users) => if midos_house_user_id.map_or(false, |user| users.contains(&user)) {
                    RoomAvailability::Open
                } else {
                    RoomAvailability::Invisible
                },
            }
        }
    }

    pub fn same_namespace(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Password { .. }, Self::Password { .. }) => true,
            (Self::Invitational(invitees1), Self::Invitational(invitees2)) => invitees1.iter().any(|invitee1| invitees2.iter().any(|invitee2| invitee1 == invitee2)),
            (Self::Password { .. }, Self::Invitational(_)) | (Self::Invitational(_), Self::Password { .. }) => false
        }
    }
}

impl fmt::Debug for RoomAuth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Password { hash: _, salt: _ } => f.debug_struct("Password")
                .field("hash", &format_args!("_"))
                .field("salt", &format_args!("_"))
                .finish(),
            Self::Invitational(users) => f.debug_tuple("Invitational")
                .field(users)
                .finish(),
        }
    }
}

#[derive(PartialEq, Eq)]
pub enum RoomAvailability {
    Open,
    PasswordRequired,
    Invisible,
}

#[derive(Derivative)]
#[derivative(Debug(bound = ""))]
pub struct Room<C: ClientKind> {
    pub id: u64,
    pub name: String,
    pub auth: RoomAuth,
    pub clients: HashMap<C::SessionId, Client<C>>,
    pub file_hash: Option<[HashIcon; 5]>,
    pub base_queue: Vec<Item>,
    pub player_queues: HashMap<NonZeroU8, Vec<Item>>,
    pub deleted: bool,
    pub last_saved: DateTime<Utc>,
    pub allow_send_all: bool,
    pub autodelete_delta: Duration,
    pub autodelete_tx: broadcast::Sender<(u64, DateTime<Utc>)>,
    #[cfg(feature = "sqlx")]
    pub db_pool: PgPool,
    pub tracker_state: Option<(String, tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>)>,
}

#[derive(Debug, thiserror::Error)]
pub enum SendItemError {
    #[error("unknown location: {0}")]
    Key(String),
    #[error("unknown item kind: {0}")]
    Kind(String),
}

#[derive(Debug, thiserror::Error)]
pub enum SendAllError {
    #[error(transparent)] Clone(#[from] ootr_utils::CloneError),
    #[error(transparent)] Dir(#[from] ootr_utils::DirError),
    #[error(transparent)] PyJson(#[from] ootr_utils::PyJsonError),
    #[error(transparent)] Room(#[from] RoomError),
    #[error("the SendAll command is not allowed in tournament rooms")]
    Disallowed,
    #[error("the given world number is not listed in the given spoiler log's locations section")]
    NoSuchWorld,
}

impl IsNetworkError for SendAllError {
    fn is_network_error(&self) -> bool {
        match self {
            Self::Clone(_) => false,
            Self::Dir(_) => false,
            Self::PyJson(_) => false,
            Self::Room(e) => e.is_network_error(),
            Self::Disallowed => false,
            Self::NoSuchWorld => false,
        }
    }
}

bitflags! {
    #[derive(Default, PartialEq, Eq)]
    struct ProgressiveItems: u32 {
        const BOMBCHU_BAG = 0x0040_0000;
        const OCARINA_OFTIME = 0x0020_0000;
        const OCARINA_FAIRY = 0x0010_0000;
        const MAGIC_DOUBLE = 0x0008_0000;
        const MAGIC_SINGLE = 0x0004_0000;
        const STICKS_30 = 0x0003_0000;
        const STICKS_20 = 0x0002_0000;
        const STICKS_10 = 0x0001_0000;
        const NUTS_40 = 0x0000_c000;
        const NUTS_30 = 0x0000_8000;
        const NUTS_20 = 0x0000_4000;
        const SCALE_GOLD = 0x0000_2000;
        const SCALE_SILVER = 0x0000_1000;
        const WALLET_999 = 0x0000_0c00;
        const WALLET_500 = 0x0000_0800;
        const WALLET_200 = 0x0000_0400;
        const SLINGSHOT_50 = 0x0000_0300;
        const SLINGSHOT_40 = 0x0000_0200;
        const SLINGSHOT_30 = 0x0000_0100;
        const BOW_50 = 0x0000_00c0;
        const BOW_40 = 0x0000_0080;
        const BOW_30 = 0x0000_0040;
        const BOMBS_40 = 0x0000_0030;
        const BOMBS_30 = 0x0000_0020;
        const BOMBS_20 = 0x0000_0010;
        const STRENGTH_3 = 0x0000_000c;
        const STRENGTH_2 = 0x0000_0008;
        const STRENGTH_1 = 0x0000_0004;
        const LONGSHOT = 0x0000_0002;
        const HOOKSHOT = 0x0000_0001;
    }
}

impl ProgressiveItems {
    fn new(save: &oottracker::Save) -> Self {
        (if save.inv.bombchus {
            Self::BOMBCHU_BAG
        } else {
            Self::default()
        }) | match save.inv.ocarina {
            oottracker::save::Ocarina::None => Self::default(),
            oottracker::save::Ocarina::FairyOcarina => Self::OCARINA_FAIRY,
            oottracker::save::Ocarina::OcarinaOfTime => Self::OCARINA_OFTIME,
        } | match save.magic {
            oottracker::save::MagicCapacity::None => Self::default(),
            oottracker::save::MagicCapacity::Small => Self::MAGIC_SINGLE,
            oottracker::save::MagicCapacity::Large => Self::MAGIC_DOUBLE,
        } | match save.upgrades.stick_capacity() {
            oottracker::save::Upgrades::DEKU_STICK_CAPACITY_30 => Self::STICKS_30,
            oottracker::save::Upgrades::DEKU_STICK_CAPACITY_20 => Self::STICKS_20,
            oottracker::save::Upgrades::DEKU_STICK_CAPACITY_10 => Self::STICKS_10,
            _ => Self::default(),
        } | match save.upgrades.nut_capacity() {
            oottracker::save::Upgrades::DEKU_NUT_CAPACITY_40 => Self::NUTS_40,
            oottracker::save::Upgrades::DEKU_NUT_CAPACITY_30 => Self::NUTS_30,
            oottracker::save::Upgrades::DEKU_NUT_CAPACITY_20 => Self::NUTS_20,
            _ => Self::default(),
        } | match save.upgrades.scale() {
            oottracker::save::Upgrades::GOLD_SCALE => Self::SCALE_GOLD,
            oottracker::save::Upgrades::SILVER_SCALE => Self::SCALE_SILVER,
            _ => Self::default(),
        } | match save.upgrades.wallet() {
            oottracker::save::Upgrades::TYCOONS_WALLET => Self::WALLET_999,
            oottracker::save::Upgrades::GIANTS_WALLET => Self::WALLET_500,
            oottracker::save::Upgrades::ADULTS_WALLET => Self::WALLET_200,
            _ => Self::default(),
        } | match save.upgrades.bullet_bag() {
            oottracker::save::Upgrades::BULLET_BAG_50 => Self::SLINGSHOT_50,
            oottracker::save::Upgrades::BULLET_BAG_40 => Self::SLINGSHOT_40,
            oottracker::save::Upgrades::BULLET_BAG_30 => Self::SLINGSHOT_30,
            _ => Self::default(),
        } | match save.upgrades.quiver() {
            oottracker::save::Upgrades::QUIVER_50 => Self::BOW_50,
            oottracker::save::Upgrades::QUIVER_40 => Self::BOW_40,
            oottracker::save::Upgrades::QUIVER_30 => Self::BOW_30,
            _ => Self::default(),
        } | match save.upgrades.bomb_bag() {
            oottracker::save::Upgrades::BOMB_BAG_40 => Self::BOMBS_40,
            oottracker::save::Upgrades::BOMB_BAG_30 => Self::BOMBS_30,
            oottracker::save::Upgrades::BOMB_BAG_20 => Self::BOMBS_20,
            _ => Self::default(),
        } | match save.upgrades.strength() {
            oottracker::save::Upgrades::GOLD_GAUNTLETS => Self::STRENGTH_3,
            oottracker::save::Upgrades::SILVER_GAUNTLETS => Self::STRENGTH_2,
            oottracker::save::Upgrades::GORON_BRACELET => Self::STRENGTH_1,
            _ => Self::default(),
        } | match save.inv.hookshot {
            oottracker::save::Hookshot::None => Self::default(),
            oottracker::save::Hookshot::Hookshot => Self::HOOKSHOT,
            oottracker::save::Hookshot::Longshot => Self::LONGSHOT,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RoomError {
    #[error(transparent)] Wheel(#[from] wheel::Error),
    #[error(transparent)] Write(#[from] async_proto::WriteError),
    #[error("this room is for a different seed: server has {} but client has {}", natjoin(.server).unwrap(), natjoin(.client).unwrap())]
    FileHash {
        server: [HashIcon; 5],
        client: [HashIcon; 5],
    },
}

impl IsNetworkError for RoomError {
    fn is_network_error(&self) -> bool {
        match self {
            Self::Wheel(e) => e.is_network_error(),
            Self::Write(e) => e.is_network_error(),
            Self::FileHash { .. } => false,
        }
    }
}

impl<C: ClientKind> Room<C> {
    async fn write(&mut self, client_id: C::SessionId, msg: unversioned::ServerMessage) -> Result<(), RoomError> {
        if let Some(client) = self.clients.get(&client_id) {
            if let Err(e) = lock!(writer = client.writer; writer.write(msg).await) {
                eprintln!("error sending message: {e} ({e:?})");
                self.remove_client(client_id, EndRoomSession::Disconnect).await?;
            }
        }
        Ok(())
    }

    async fn write_all(&mut self, msg: &unversioned::ServerMessage) -> Result<(), RoomError> {
        let mut notified = HashSet::new();
        while let Some((&client_id, client)) = self.clients.iter().find(|&(client_id, _)| !notified.contains(client_id)) {
            if let Err(e) = lock!(writer = client.writer; writer.write(msg.clone()).await) {
                eprintln!("error sending message: {e} ({e:?})");
                self.remove_client(client_id, EndRoomSession::Disconnect).await?;
            }
            notified.insert(client_id);
        }
        Ok(())
    }

    pub async fn add_client(&mut self, client_id: C::SessionId, writer: Arc<Mutex<C::Writer>>, end_tx: oneshot::Sender<EndRoomSession>) -> Result<(), RoomError> {
        // the client doesn't need to be told that it has connected, so notify everyone *before* adding it
        self.write_all(&unversioned::ServerMessage::ClientConnected).await?;
        self.clients.insert(client_id, Client {
            player: None,
            pending_world: None,
            pending_name: None,
            pending_hash: None,
            pending_items: Vec::default(),
            tracker_state: oottracker::ModelState::default(),
            adjusted_save: oottracker::Save::default(),
            writer, end_tx,
        });
        Ok(())
    }

    pub fn has_client(&self, client_id: C::SessionId) -> bool {
        self.clients.contains_key(&client_id)
    }

    pub async fn remove_client(&mut self, client_id: C::SessionId, to: EndRoomSession) -> Result<(), RoomError> {
        if let Some(client) = self.clients.remove(&client_id) {
            let _ = client.end_tx.send(to);
            let msg = if let Some(Player { world, .. }) = client.player {
                if let Some((&client_id, _)) = self.clients.iter().find(|(_, iter_client)| iter_client.pending_world == Some(world)) {
                    Box::pin(self.load_player(client_id, world)).await?;
                }
                unversioned::ServerMessage::PlayerDisconnected(world)
            } else {
                unversioned::ServerMessage::UnregisteredClientDisconnected
            };
            Box::pin(self.write_all(&msg)).await?;
        }
        Ok(())
    }

    pub async fn delete(&mut self) -> Result<(), RoomError> {
        self.deleted = true;
        for client_id in self.clients.keys().copied().collect::<Vec<_>>() {
            self.remove_client(client_id, EndRoomSession::ToLobby).await?;
        }
        #[cfg(feature = "sqlx")] {
            if let Err(e) = sqlx::query!("DELETE FROM mw_rooms WHERE id = $1", self.id as i64).execute(&self.db_pool).await {
                eprintln!("failed to delete room from database: {e} ({e:?})");
                wheel::night_report("/games/zelda/oot/mhmw/error", Some(&format!("failed to delete room from database: {e} ({e:?})"))).await?;
            }
        }
        if let Some((ref tracker_room_name, ref mut sock)) = self.tracker_state {
            let _ = oottracker::websocket::ClientMessage::MwDeleteRoom { room: tracker_room_name.clone() }.write_ws(sock).await;
        }
        Ok(())
    }

    /// Moves a player from unloaded (no world assigned) to the given `world`.
    pub async fn load_player(&mut self, client_id: C::SessionId, world: NonZeroU8) -> Result<bool, RoomError> {
        if self.clients.iter().any(|(&iter_client_id, iter_client)| iter_client.player.as_ref().map_or(false, |p| p.world == world) && iter_client_id != client_id) {
            let client = self.clients.get_mut(&client_id).expect("tried to set pending world for nonexistent client");
            client.pending_world = Some(world);
            return Ok(false)
        }
        let client = self.clients.get_mut(&client_id).expect("tried to set world for nonexistent client");
        let save = client.tracker_state.ram.save.clone();
        let prev_player = &mut client.player;
        if let Some(player) = prev_player {
            let prev_world = mem::replace(&mut player.world, world);
            if prev_world == world { return Ok(true) }
            self.write_all(&unversioned::ServerMessage::ResetPlayerId(prev_world)).await?;
            self.write_all(&unversioned::ServerMessage::PlayerId(world)).await?;
        } else {
            let mut new_player = Player::new(world);
            let mut broadcasts = vec![unversioned::ServerMessage::PlayerId(world)];
            if let Some(name) = client.pending_name.take() {
                new_player.name = name;
                broadcasts.push(unversioned::ServerMessage::PlayerName(world, name));
            }
            if let Some(player_hash) = client.pending_hash.take() {
                if let Some(room_hash) = self.file_hash {
                    if player_hash != room_hash {
                        return Err(RoomError::FileHash { server: room_hash, client: player_hash })
                    }
                } else {
                    self.file_hash = Some(player_hash);
                }
                new_player.file_hash = Some(player_hash);
                broadcasts.push(unversioned::ServerMessage::PlayerFileHash(world, player_hash));
            }
            let pending_items = mem::take(&mut client.pending_items);
            *prev_player = Some(new_player);
            if client.pending_world.take().is_some() {
                self.write(client_id, unversioned::ServerMessage::WorldFreed).await?;
            }
            for (key, kind, target_world) in pending_items {
                self.queue_item_inner(Some(client_id), new_player.world, key, kind, target_world, "while queueing a pending item", false).await?;
            }
            for broadcast in broadcasts {
                self.write_all(&broadcast).await?;
            }
        }
        let queue = self.player_queues.get(&world).unwrap_or(&self.base_queue).iter().map(|item| item.kind).collect::<Vec<_>>();
        let mut adjusted_save = save.clone();
        if let Some(queued_items) = queue.get(adjusted_save.inv_amounts.num_received_mw_items.into()..) {
            for &item in queued_items {
                if let Err(()) = adjusted_save.recv_mw_item(item) {
                    eprintln!("load_player: item 0x{item:04x} not supported by recv_mw_item");
                    wheel::night_report("/games/zelda/oot/mhmw/error", Some(&format!("load_player: item 0x{item:04x} not supported by recv_mw_item"))).await?;
                }
            }
        } else {
            eprintln!("save data from player {world} in room {} has more received items than are in their queue", self.name);
        }
        if let Some(client) = self.clients.get_mut(&client_id) {
            let old_progressive_items = ProgressiveItems::new(&client.adjusted_save);
            let new_progressive_items = ProgressiveItems::new(&adjusted_save);
            client.adjusted_save = adjusted_save;
            if old_progressive_items != new_progressive_items {
                self.write_all(&unversioned::ServerMessage::ProgressiveItems { world, state: new_progressive_items.bits() }).await?;
            }
        }
        if !queue.is_empty() {
            self.write(client_id, unversioned::ServerMessage::ItemQueue(queue)).await?;
        }
        if let Some((ref tracker_room_name, ref mut sock)) = self.tracker_state {
            oottracker::websocket::ClientMessage::MwResetPlayer { room: tracker_room_name.clone(), world, save }.write_ws(sock).await?;
        }
        Ok(true)
    }

    pub async fn unload_player(&mut self, client_id: C::SessionId) -> Result<(), RoomError> {
        if let Some(prev_player) = self.clients.get_mut(&client_id).expect("tried to unset world for nonexistent client").player.take() {
            self.write_all(&unversioned::ServerMessage::ResetPlayerId(prev_player.world)).await?;
            if let Some((&client_id, _)) = self.clients.iter().find(|(_, iter_client)| iter_client.pending_world == Some(prev_player.world)) {
                self.load_player(client_id, prev_player.world).await?;
            }
        }
        Ok(())
    }

    pub async fn set_player_name(&mut self, client_id: C::SessionId, name: Filename) -> Result<(), RoomError> {
        let client = self.clients.get_mut(&client_id).expect("tried to set filename for nonexsitent client");
        if let Some(ref mut player) = client.player {
            let world = player.world;
            player.name = name;
            self.write_all(&unversioned::ServerMessage::PlayerName(world, name)).await?;
        } else {
            client.pending_name = Some(name);
        }
        Ok(())
    }

    pub async fn set_file_hash(&mut self, client_id: C::SessionId, hash: [HashIcon; 5]) -> Result<(), RoomError> {
        if let Some(room_hash) = self.file_hash {
            if room_hash != hash {
                return Err(RoomError::FileHash { server: room_hash, client: hash })
            }
        }
        let client = self.clients.get_mut(&client_id).expect("tried to set file hash for nonexistent client");
        if let Some(ref mut player) = client.player {
            let world = player.world;
            player.file_hash = Some(hash);
            self.write_all(&unversioned::ServerMessage::PlayerFileHash(world, hash)).await?;
        } else {
            client.pending_hash = Some(hash);
        }
        Ok(())
    }

    async fn queue_item_inner(&mut self, source_client: Option<C::SessionId>, source_world: NonZeroU8, key: u64, kind: u16, target_world: NonZeroU8, #[cfg_attr(not(feature = "sqlx"), allow(unused))] context: &str, verbose_logging: bool) -> Result<(), RoomError> {
        if let Some((ref tracker_room_name, ref mut sock)) = self.tracker_state {
            if verbose_logging { println!("updating tracker") }
            oottracker::websocket::ClientMessage::MwQueueItem {
                room: tracker_room_name.clone(),
                source_world, key, kind, target_world,
            }.write_ws(sock).await?;
            if verbose_logging { println!("tracker updated") }
        } else {
            if verbose_logging { println!("no tracker room") }
        }
        if kind == TRIFORCE_PIECE {
            if verbose_logging { println!("is Triforce piece") }
            if !self.base_queue.iter().any(|item| item.source == source_world && item.key == key) {
                self.player_queues.entry(source_world).or_insert_with(|| self.base_queue.clone()); // make sure the sender doesn't get a duplicate of this piece from the base queue
                let item = Item { source: source_world, key, kind };
                self.base_queue.push(item);
                for (&target_world, queue) in &mut self.player_queues {
                    if source_world != target_world {
                        queue.push(item);
                    }
                }
                let msg = unversioned::ServerMessage::GetItem(kind);
                let player_clients = self.clients.iter()
                    .filter_map(|(&target_client, c)| if c.player.map_or(false, |p| p.world != source_world) { Some(target_client) } else { None })
                    .collect::<Vec<_>>();
                for target_client in player_clients {
                    self.write(target_client, msg.clone()).await?;
                }
            }
        } else if source_world == target_world {
            if verbose_logging { println!("is own world") }
            let mut changed_progressive_items = Vec::default();
            for client in self.clients.values_mut() {
                if client.player.map_or(false, |p| p.world == target_world) {
                    let old_progressive_items = ProgressiveItems::new(&client.adjusted_save);
                    if let Err(()) = client.adjusted_save.recv_mw_item(kind) {
                        eprintln!("queue_item_inner (own world): item 0x{kind:04x} not supported by recv_mw_item");
                        wheel::night_report("/games/zelda/oot/mhmw/error", Some(&format!("queue_item_inner (own world): item 0x{kind:04x} not supported by recv_mw_item"))).await?;
                    }
                    let new_progressive_items = ProgressiveItems::new(&client.adjusted_save);
                    if old_progressive_items != new_progressive_items {
                        changed_progressive_items.push((target_world, new_progressive_items.bits()));
                    }
                }
            }
            for (world, state) in changed_progressive_items {
                self.write_all(&unversioned::ServerMessage::ProgressiveItems { world, state }).await?;
            }
            // don't send own item back to sender
        } else {
            if verbose_logging { println!("regular item send") }
            if let Some(&Item { kind: existing_kind, .. }) = self.player_queues.get(&target_world).and_then(|queue| queue.iter().find(|item| item.source == source_world && item.key == key)) {
                if kind == existing_kind {
                    if verbose_logging { println!("item is a duplicate") }
                } else {
                    eprintln!("conflicting item kinds at location 0x{key:016x} from world {source_world} in room {:?}: sent earlier as 0x{existing_kind:04x}, now as 0x{kind:04x}", self.name);
                    if let Some(source_client) = source_client {
                        self.write(source_client, unversioned::ServerMessage::StructuredError(ServerError::ConflictingItemKinds)).await?;
                    }
                }
            } else {
                if verbose_logging { println!("item not a duplicate") }
                self.player_queues.entry(target_world).or_insert_with(|| self.base_queue.clone()).push(Item { source: source_world, key, kind });
                if let Some((&target_client, client)) = self.clients.iter_mut().find(|(_, c)| c.player.is_some_and(|p| p.world == target_world)) {
                    if verbose_logging { println!("target is connected") }
                    let old_progressive_items = ProgressiveItems::new(&client.adjusted_save);
                    if let Err(()) = client.adjusted_save.recv_mw_item(kind) {
                        eprintln!("queue_item_inner (cross world): item 0x{kind:04x} not supported by recv_mw_item");
                        wheel::night_report("/games/zelda/oot/mhmw/error", Some(&format!("queue_item_inner (cross world): item 0x{kind:04x} not supported by recv_mw_item"))).await?;
                    }
                    let new_progressive_items = ProgressiveItems::new(&client.adjusted_save);
                    self.write(target_client, unversioned::ServerMessage::GetItem(kind)).await?;
                    if old_progressive_items != new_progressive_items {
                        if verbose_logging { println!("updating progressive items") }
                        self.write_all(&unversioned::ServerMessage::ProgressiveItems { world: target_world, state: new_progressive_items.bits() }).await?;
                    } else {
                        if verbose_logging { println!("no progressive items change") }
                    }
                } else {
                    if verbose_logging { println!("target not connected") }
                }
            }
        }
        #[cfg(feature = "sqlx")] {
            if let Err(e) = self.save(true).await {
                eprintln!("failed to save room state while trying to queue item for room {} {context} ({}): {e} ({e:?})", self.name, self.id);
                wheel::night_report("/games/zelda/oot/mhmw/error", Some(&format!("failed to save room state while trying to queue item for room {} {context} ({}): {e} ({e:?})", self.name, self.id))).await?;
            } else {
                if verbose_logging { println!("database updated") }
            }
        }
        Ok(())
    }

    pub async fn queue_item(&mut self, source_client_id: C::SessionId, key: u64, kind: u16, target_world: NonZeroU8, verbose_logging: bool) -> Result<(), RoomError> {
        let source_client = self.clients.get_mut(&source_client_id).expect("tried to queue item from nonexistent client");
        if let Some(source) = source_client.player {
            if let Some(player_hash) = source.file_hash {
                if let Some(room_hash) = self.file_hash {
                    if player_hash != room_hash {
                        return Err(RoomError::FileHash { server: room_hash, client: player_hash })
                    }
                } else {
                    self.file_hash = Some(player_hash);
                }
            }
            self.queue_item_inner(Some(source_client_id), source.world, key, kind, target_world, "while queueing an item", verbose_logging).await?;
        } else {
            source_client.pending_items.push((key, kind, target_world));
        }
        Ok(())
    }

    pub async fn send_all(&mut self, source_world: NonZeroU8, spoiler_log: &SpoilerLog, logged_in_as_admin: bool) -> Result<(), SendAllError> {
        if !self.allow_send_all && !logged_in_as_admin {
            return Err(SendAllError::Disallowed)
        }
        if let Some(room_hash) = self.file_hash {
            if spoiler_log.file_hash != room_hash {
                return Err(SendAllError::Room(RoomError::FileHash { server: room_hash, client: spoiler_log.file_hash }))
            }
        } else {
            self.file_hash = Some(spoiler_log.file_hash);
        }
        spoiler_log.version.clone_repo().await?;
        let py_modules = spoiler_log.version.py_modules("/usr/bin/python3")?;
        let mut items_to_queue = Vec::default();
        let world_locations = spoiler_log.locations.get(usize::from(source_world.get() - 1)).ok_or(SendAllError::NoSuchWorld)?;
        for (loc, ootr_utils::spoiler::Item { player, item, model: _ }) in world_locations {
            if let Some((key, kind)) = py_modules.override_entry(source_world, loc, *player, item, spoiler_log.settings.get(usize::from(player.get() - 1)).unwrap_or_else(|| &spoiler_log.settings[0]).keyring_give_bk).await? {
                if kind == TRIFORCE_PIECE || *player != source_world {
                    items_to_queue.push((source_world, key, kind, *player));
                }
            }
        }
        for (source_world, key, kind, target_world) in items_to_queue {
            self.queue_item_inner(None, source_world, key, kind, target_world, "while sending all items", false).await?;
        }
        Ok(())
    }

    pub async fn set_save_data(&mut self, client_id: C::SessionId, save: oottracker::Save) -> Result<(), RoomError> {
        let client = self.clients.get_mut(&client_id).expect("tried to set save data for nonexistent client");
        client.tracker_state.ram.save = save.clone();
        if let Some(Player { world, .. }) = client.player {
            let queue = self.player_queues.get(&world).unwrap_or(&self.base_queue).iter().map(|item| item.kind).collect::<Vec<_>>();
            let mut adjusted_save = save.clone();
            if let Some(queued_items) = queue.get(adjusted_save.inv_amounts.num_received_mw_items.into()..) {
                for &item in queued_items {
                    if let Err(()) = adjusted_save.recv_mw_item(item) {
                        eprintln!("set_save_data: item 0x{item:04x} not supported by recv_mw_item");
                        wheel::night_report("/games/zelda/oot/mhmw/error", Some(&format!("set_save_data: item 0x{item:04x} not supported by recv_mw_item"))).await?;
                    }
                }
            } else {
                eprintln!("save data from client has more received items than are in their queue");
            }
            let old_progressive_items = ProgressiveItems::new(&client.adjusted_save);
            let new_progressive_items = ProgressiveItems::new(&adjusted_save);
            client.adjusted_save = adjusted_save;
            if old_progressive_items != new_progressive_items {
                self.write_all(&unversioned::ServerMessage::ProgressiveItems { world, state: new_progressive_items.bits() }).await?;
            }
            if let Some((ref tracker_room_name, ref mut sock)) = self.tracker_state {
                oottracker::websocket::ClientMessage::MwResetPlayer { room: tracker_room_name.clone(), world, save }.write_ws(sock).await?;
            }
        }
        Ok(())
    }

    pub async fn add_dungeon_reward_info(&mut self, client_id: C::SessionId, reward: DungeonReward, _ /*source_world*/ /*TODO for dungeon reward shuffle, track which world the reward is in */: NonZeroU8, location: DungeonRewardLocation) -> Result<(), async_proto::WriteError> {
        let client = self.clients.get_mut(&client_id).expect("tried to add dungeon reward info for nonexistent client");
        client.tracker_state.knowledge.dungeon_reward_locations.insert(reward, location);
        if let Some(Player { world, .. }) = client.player {
            if let Some((ref tracker_room_name, ref mut sock)) = self.tracker_state {
                oottracker::websocket::ClientMessage::MwDungeonRewardLocation { room: tracker_room_name.clone(), world, reward, location }.write_ws(sock).await?;
            }
        }
        Ok(())
    }

    pub async fn init_tracker(&mut self, tracker_room_name: String, world_count: NonZeroU8) -> Result<(), async_proto::WriteError> {
        let mut worlds = (1..=world_count.get())
            .map(|player_id| (
                oottracker::ModelState::default(),
                self.player_queues.get(&NonZeroU8::new(player_id).expect("range starts at 1")).unwrap_or(&self.base_queue).clone(),
            ))
            .collect::<Vec<_>>();
        for client in self.clients.values() {
            if let Some(player) = client.player {
                worlds[usize::from(player.world.get() - 1)].0 = client.tracker_state.clone();
            }
        }
        let mut sock = tokio_tungstenite::connect_async("wss://oottracker.fenhl.net/websocket").await.map_err(|e| async_proto::WriteError {
            context: async_proto::ErrorContext::Custom(format!("multiworld::Room::init_tracker")),
            kind: e.into(),
        })?.0;
        oottracker::websocket::ClientMessage::MwCreateRoom { room: tracker_room_name.clone(), worlds }.write_ws(&mut sock).await?;
        self.tracker_state = Some((tracker_room_name, sock));
        Ok(())
    }

    pub fn autodelete_at(&self) -> DateTime<Utc> {
        self.last_saved + TimeDelta::from_std(self.autodelete_delta).expect("autodelete delta too long")
    }

    #[cfg(feature = "sqlx")]
    pub async fn save(&mut self, update_last_saved: bool) -> sqlx::Result<()> {
        if self.deleted { return Ok(()) }
        let mut base_queue = Vec::default();
        self.base_queue.write_sync(&mut base_queue).expect("failed to write base queue to buffer");
        let mut player_queues = Vec::default();
        self.player_queues.write_sync(&mut player_queues).expect("failed to write player queues to buffer");
        if update_last_saved {
            self.last_saved = Utc::now();
            let _ = self.autodelete_tx.send((self.id, self.autodelete_at()));
        }
        let (password_hash, password_salt, invites) = match self.auth {
            RoomAuth::Password { ref hash, ref salt } => (Some(&hash[..]), Some(&salt[..]), Vec::default()),
            RoomAuth::Invitational(ref invites) => {
                let mut buf = Vec::default();
                invites.write_sync(&mut buf).expect("failed to write invites to buffer");
                (None, None, buf)
            },
        };
        sqlx::query!("INSERT INTO mw_rooms (
            id,
            name,
            password_hash,
            password_salt,
            invites,
            base_queue,
            player_queues,
            last_saved,
            autodelete_delta,
            allow_send_all
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) ON CONFLICT (id) DO UPDATE SET
            name = EXCLUDED.name,
            password_hash = EXCLUDED.password_hash,
            password_salt = EXCLUDED.password_salt,
            invites = EXCLUDED.invites,
            base_queue = EXCLUDED.base_queue,
            player_queues = EXCLUDED.player_queues,
            last_saved = EXCLUDED.last_saved,
            autodelete_delta = EXCLUDED.autodelete_delta,
            allow_send_all = EXCLUDED.allow_send_all
        ", self.id as i64, &self.name, password_hash, password_salt, invites, base_queue, player_queues, self.last_saved, self.autodelete_delta as _, self.allow_send_all).execute(&self.db_pool).await?;
        Ok(())
    }

    pub async fn set_autodelete_delta(&mut self, new_delta: Duration) -> Result<(), RoomError> {
        self.autodelete_delta = new_delta;
        #[cfg(feature = "sqlx")] {
            // saving also notifies the room deletion waiter
            if let Err(e) = self.save(true).await {
                eprintln!("failed to save room state while trying to set autodelete delta for room {} ({}): {e} ({e:?})", self.name, self.id);
                wheel::night_report("/games/zelda/oot/mhmw/error", Some(&format!("failed to save room state while trying to set autodelete delta for room {} ({}): {e} ({e:?}", self.name, self.id))).await?;
            }
        }
        self.write_all(&unversioned::ServerMessage::AutoDeleteDelta(new_delta)).await?;
        Ok(())
    }
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
    Mismatch {
        expected: &'static str,
        actual: Box<SessionState<E>>,
    },
    #[error("server error: {0}")]
    Server(String),
}

#[derive(Debug, Default, Clone, Copy)]
pub struct LoginState {
    pub admin: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum IdentityProvider {
    RaceTime,
    Discord,
}

impl fmt::Display for IdentityProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RaceTime => write!(f, "racetime.gg"),
            Self::Discord => write!(f, "Discord"),
        }
    }
}

#[derive(Debug, Clone)]
pub enum LobbyView {
    Normal,
    SessionExpired {
        provider: IdentityProvider,
        error: Option<Arc<oauth2::basic::BasicRequestTokenError<oauth2::reqwest::HttpClientError>>>,
    },
    Settings,
    Login {
        provider: IdentityProvider,
        no_midos_house_account: bool,
    },
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
        maintenance: Option<(DateTime<Utc>, Duration)>,
        e: SessionStateError<E>,
        auto_retry: bool,
    },
    Init {
        maintenance: Option<(DateTime<Utc>, Duration)>,
    },
    InitAutoRejoin {
        maintenance: Option<(DateTime<Utc>, Duration)>,
        room_id: u64,
        room_password: String,
    },
    Lobby {
        maintenance: Option<(DateTime<Utc>, Duration)>,
        login_state: Option<LoginState>,
        rooms: BTreeMap<u64, (String, bool)>,
        create_new_room: bool,
        existing_room_selection: Option<RoomFormatter>,
        new_room_name: String,
        password: String,
        view: LobbyView,
        wrong_password: bool,
    },
    Room {
        maintenance: Option<(DateTime<Utc>, Duration)>,
        login_state: Option<LoginState>,
        room_id: u64,
        room_name: String,
        room_password: String,
        players: Vec<Player>,
        progressive_items: HashMap<NonZeroU8, u32>,
        num_unassigned_clients: u8,
        item_queue: Vec<u16>,
        autodelete_delta: Duration,
        allow_send_all: bool,
        view: RoomView,
        wrong_file_hash: Option<[[HashIcon; 5]; 2]>,
        world_taken: Option<NonZeroU8>,
        conflicting_item_kinds: bool,
    },
    Closed {
        maintenance: Option<(DateTime<Utc>, Duration)>,
    },
}

impl<E> SessionState<E> {
    pub fn maintenance(&self) -> Option<(DateTime<Utc>, Duration)> {
        match *self {
            | Self::Error { maintenance, .. }
            | Self::Init { maintenance, .. }
            | Self::InitAutoRejoin { maintenance, .. }
            | Self::Lobby { maintenance, .. }
            | Self::Room { maintenance, .. }
            | Self::Closed { maintenance, .. }
                => maintenance,
        }
    }

    pub fn apply(&mut self, msg: latest::ServerMessage) {
        match msg {
            latest::ServerMessage::Ping => {}
            latest::ServerMessage::StructuredError(ServerError::WrongPassword) => if let Self::Lobby { password, wrong_password, .. } = self {
                *wrong_password = true;
                password.clear();
            } else {
                *self = Self::Error {
                    maintenance: self.maintenance(),
                    e: SessionStateError::Mismatch {
                        expected: "Lobby",
                        actual: Box::new(mem::replace(self, Self::Init { maintenance: self.maintenance() })),
                    },
                    auto_retry: false,
                };
            },
            latest::ServerMessage::StructuredError(ServerError::NoMidosHouseAccountDiscord) => if let Self::Lobby { view: LobbyView::Login { provider: IdentityProvider::Discord, no_midos_house_account }, .. } = self {
                *no_midos_house_account = true;
            } else {
                // ignore, GUI code should delete login token
            },
            latest::ServerMessage::StructuredError(ServerError::NoMidosHouseAccountRaceTime) => if let Self::Lobby { view: LobbyView::Login { provider: IdentityProvider::RaceTime, no_midos_house_account }, .. } = self {
                *no_midos_house_account = true;
            } else {
                // ignore, GUI code should delete login token
            },
            latest::ServerMessage::StructuredError(ServerError::SessionExpiredDiscord | ServerError::SessionExpiredRaceTime) => {
                // ignore, GUI code should refresh login token
            }
            latest::ServerMessage::WrongFileHash { server, client } => if let Self::Room { wrong_file_hash, .. } = self {
                *wrong_file_hash = Some([server, client]);
            } else {
                *self = Self::Error {
                    maintenance: self.maintenance(),
                    e: SessionStateError::Mismatch {
                        expected: "Room",
                        actual: Box::new(mem::replace(self, Self::Init { maintenance: self.maintenance() })),
                    },
                    auto_retry: false,
                };
            },
            latest::ServerMessage::WorldTaken(world) => if let Self::Room { world_taken, .. } = self {
                *world_taken = Some(world);
            } else {
                *self = Self::Error {
                    maintenance: self.maintenance(),
                    e: SessionStateError::Mismatch {
                        expected: "Room",
                        actual: Box::new(mem::replace(self, Self::Init { maintenance: self.maintenance() })),
                    },
                    auto_retry: false,
                };
            },
            latest::ServerMessage::WorldFreed => if let Self::Room { world_taken, .. } = self {
                *world_taken = None;
            } else {
                *self = Self::Error {
                    maintenance: self.maintenance(),
                    e: SessionStateError::Mismatch {
                        expected: "Room",
                        actual: Box::new(mem::replace(self, Self::Init { maintenance: self.maintenance() })),
                    },
                    auto_retry: false,
                };
            },
            latest::ServerMessage::StructuredError(ServerError::RoomExists) => if let Self::Lobby { create_new_room: ref mut create_new_room @ true, .. } = self {
                *create_new_room = false;
            } else {
                *self = Self::Error {
                    maintenance: self.maintenance(),
                    e: SessionStateError::Mismatch {
                        expected: "Lobby",
                        actual: Box::new(mem::replace(self, Self::Init { maintenance: self.maintenance() })),
                    },
                    auto_retry: false,
                };
            },
            latest::ServerMessage::StructuredError(ServerError::ConflictingItemKinds) => if let Self::Room { ref mut conflicting_item_kinds, .. } = self {
                *conflicting_item_kinds = true; //TODO update client to automatically send additional seed info when receiving this?
            } else {
                *self = Self::Error {
                    maintenance: self.maintenance(),
                    e: SessionStateError::Mismatch {
                        expected: "Room",
                        actual: Box::new(mem::replace(self, Self::Init { maintenance: self.maintenance() })),
                    },
                    auto_retry: false,
                };
            },
            latest::ServerMessage::StructuredError(ServerError::Future(discrim)) => if !matches!(self, Self::Error { .. }) {
                *self = Self::Error {
                    maintenance: self.maintenance(),
                    e: SessionStateError::Future(discrim),
                    auto_retry: false,
                };
            },
            latest::ServerMessage::OtherError(e) => if !matches!(self, Self::Error { .. }) {
                *self = Self::Error {
                    maintenance: self.maintenance(),
                    e: SessionStateError::Server(e),
                    auto_retry: false,
                };
            },
            latest::ServerMessage::EnterLobby { rooms } => {
                let maintenance = self.maintenance();
                let login_state = match self {
                    Self::Lobby { login_state, .. } |
                    Self::Room { login_state, .. } => login_state.clone(),
                    Self::Error { .. } | Self::Init { .. } | Self::InitAutoRejoin { .. } | Self::Closed { .. } => None,
                };
                *self = if let Self::InitAutoRejoin { room_id, room_password, .. } = self {
                    let existing_room_selection = rooms.iter().find(|(&id, _)| id == *room_id).map(|(&id, (name, password_required))| RoomFormatter { id, password_required: *password_required, name: name.clone(), active: true });
                    Self::Lobby {
                        create_new_room: existing_room_selection.is_none(),
                        new_room_name: String::default(),
                        password: room_password.clone(),
                        view: LobbyView::Normal,
                        wrong_password: false,
                        maintenance, login_state, rooms, existing_room_selection,
                    }
                } else {
                    Self::Lobby {
                        create_new_room: rooms.is_empty(),
                        existing_room_selection: None,
                        new_room_name: String::default(),
                        password: String::default(),
                        view: LobbyView::Normal,
                        wrong_password: false,
                        maintenance, login_state, rooms,
                    }
                };
            }
            latest::ServerMessage::NewRoom { id, name, password_required } => if let Self::Lobby { rooms, .. } = self {
                rooms.insert(id, (name, password_required));
            } else {
                *self = Self::Error {
                    maintenance: self.maintenance(),
                    e: SessionStateError::Mismatch {
                        expected: "Lobby",
                        actual: Box::new(mem::replace(self, Self::Init { maintenance: self.maintenance() })),
                    },
                    auto_retry: false,
                };
            },
            latest::ServerMessage::DeleteRoom(id) => if let Self::Lobby { rooms, existing_room_selection, .. } = self {
                rooms.remove(&id);
                if existing_room_selection.as_ref().map_or(false, |existing_room_selection| existing_room_selection.id == id) {
                    *existing_room_selection = None;
                }
            } else {
                *self = Self::Error {
                    maintenance: self.maintenance(),
                    e: SessionStateError::Mismatch {
                        expected: "Lobby",
                        actual: Box::new(mem::replace(self, Self::Init { maintenance: self.maintenance() })),
                    },
                    auto_retry: false,
                };
            },
            latest::ServerMessage::EnterRoom { room_id, players, num_unassigned_clients, autodelete_delta, allow_send_all } => {
                let maintenance = self.maintenance();
                if let Self::Lobby { login_state, rooms, password, new_room_name, .. } = self {
                    let room_name = if let Some((_, (room_name, _))) = rooms.iter().find(|&(&id, _)| id == room_id) {
                        room_name.clone()
                    } else {
                        new_room_name.clone()
                    };
                    *self = Self::Room {
                        login_state: login_state.clone(),
                        room_name: room_name.clone(),
                        room_password: password.clone(),
                        progressive_items: HashMap::default(),
                        item_queue: Vec::default(),
                        view: RoomView::Normal,
                        wrong_file_hash: None,
                        world_taken: None,
                        conflicting_item_kinds: false,
                        maintenance, room_id, players, num_unassigned_clients, autodelete_delta, allow_send_all,
                    };
                } else {
                    *self = Self::Error {
                        e: SessionStateError::Mismatch {
                            expected: "Lobby",
                            actual: Box::new(mem::replace(self, Self::Init { maintenance: self.maintenance() })),
                        },
                        auto_retry: false,
                        maintenance,
                    };
                }
            }
            latest::ServerMessage::PlayerId(world) => if let Self::Room { players, num_unassigned_clients, .. } = self {
                if let Err(idx) = players.binary_search_by_key(&world, |p| p.world) {
                    players.insert(idx, Player::new(world));
                    *num_unassigned_clients -= 1;
                }
            } else {
                *self = Self::Error {
                    maintenance: self.maintenance(),
                    e: SessionStateError::Mismatch {
                        expected: "Room",
                        actual: Box::new(mem::replace(self, Self::Init { maintenance: self.maintenance() })),
                    },
                    auto_retry: false,
                };
            },
            latest::ServerMessage::ResetPlayerId(world) => if let Self::Room { players, num_unassigned_clients, .. } = self {
                if let Ok(idx) = players.binary_search_by_key(&world, |p| p.world) {
                    players.remove(idx);
                    *num_unassigned_clients += 1;
                }
            } else {
                *self = Self::Error {
                    maintenance: self.maintenance(),
                    e: SessionStateError::Mismatch {
                        expected: "Room",
                        actual: Box::new(mem::replace(self, Self::Init { maintenance: self.maintenance() })),
                    },
                    auto_retry: false,
                };
            },
            latest::ServerMessage::ClientConnected => if let Self::Room { num_unassigned_clients, .. } = self {
                *num_unassigned_clients += 1;
            } else {
                *self = Self::Error {
                    maintenance: self.maintenance(),
                    e: SessionStateError::Mismatch {
                        expected: "Room",
                        actual: Box::new(mem::replace(self, Self::Init { maintenance: self.maintenance() })),
                    },
                    auto_retry: false,
                };
            },
            latest::ServerMessage::PlayerDisconnected(world) => if let Self::Room { players, .. } = self {
                if let Ok(idx) = players.binary_search_by_key(&world, |p| p.world) {
                    players.remove(idx);
                }
            } else {
                *self = Self::Error {
                    maintenance: self.maintenance(),
                    e: SessionStateError::Mismatch {
                        expected: "Room",
                        actual: Box::new(mem::replace(self, Self::Init { maintenance: self.maintenance() })),
                    },
                    auto_retry: false,
                };
            },
            latest::ServerMessage::UnregisteredClientDisconnected => if let Self::Room { num_unassigned_clients, .. } = self {
                *num_unassigned_clients -= 1;
            } else {
                *self = Self::Error {
                    maintenance: self.maintenance(),
                    e: SessionStateError::Mismatch {
                        expected: "Room",
                        actual: Box::new(mem::replace(self, Self::Init { maintenance: self.maintenance() })),
                    },
                    auto_retry: false,
                };
            },
            latest::ServerMessage::PlayerName(world, name) => if let Self::Room { players, .. } = self {
                if let Ok(idx) = players.binary_search_by_key(&world, |p| p.world) {
                    players[idx].name = name;
                }
            } else {
                *self = Self::Error {
                    maintenance: self.maintenance(),
                    e: SessionStateError::Mismatch {
                        expected: "Room",
                        actual: Box::new(mem::replace(self, Self::Init { maintenance: self.maintenance() })),
                    },
                    auto_retry: false,
                };
            },
            latest::ServerMessage::ItemQueue(queue) => if let Self::Room { item_queue, .. } = self {
                *item_queue = queue.clone();
            } else {
                *self = Self::Error {
                    maintenance: self.maintenance(),
                    e: SessionStateError::Mismatch {
                        expected: "Room",
                        actual: Box::new(mem::replace(self, Self::Init { maintenance: self.maintenance() })),
                    },
                    auto_retry: false,
                };
            },
            latest::ServerMessage::GetItem(item) => if let Self::Room { item_queue, .. } = self {
                item_queue.push(item);
            } else {
                *self = Self::Error {
                    maintenance: self.maintenance(),
                    e: SessionStateError::Mismatch {
                        expected: "Room",
                        actual: Box::new(mem::replace(self, Self::Init { maintenance: self.maintenance() })),
                    },
                    auto_retry: false,
                };
            },
            latest::ServerMessage::AdminLoginSuccess { .. } => if let Self::Lobby { login_state, .. } = self {
                login_state.get_or_insert_with(LoginState::default).admin = true;
            } else {
                *self = Self::Error {
                    maintenance: self.maintenance(),
                    e: SessionStateError::Mismatch {
                        expected: "Lobby",
                        actual: Box::new(mem::replace(self, Self::Init { maintenance: self.maintenance() })),
                    },
                    auto_retry: false,
                };
            },
            latest::ServerMessage::Goodbye => if !matches!(self, Self::Error { .. }) {
                *self = Self::Closed { maintenance: self.maintenance() };
            },
            latest::ServerMessage::PlayerFileHash(world, hash) => if let Self::Room { players, .. } = self {
                if let Ok(idx) = players.binary_search_by_key(&world, |p| p.world) {
                    players[idx].file_hash = Some(hash);
                }
            } else {
                *self = Self::Error {
                    maintenance: self.maintenance(),
                    e: SessionStateError::Mismatch {
                        expected: "Room",
                        actual: Box::new(mem::replace(self, Self::Init { maintenance: self.maintenance() })),
                    },
                    auto_retry: false,
                };
            },
            latest::ServerMessage::AutoDeleteDelta(new_delta) => if let Self::Room { autodelete_delta, .. } = self {
                *autodelete_delta = new_delta;
            } else {
                *self = Self::Error {
                    maintenance: self.maintenance(),
                    e: SessionStateError::Mismatch {
                        expected: "Room",
                        actual: Box::new(mem::replace(self, Self::Init { maintenance: self.maintenance() })),
                    },
                    auto_retry: false,
                };
            },
            latest::ServerMessage::RoomsEmpty => {}
            latest::ServerMessage::ProgressiveItems { world, state } => if let Self::Room { progressive_items, .. } = self {
                progressive_items.insert(world, state);
            } else {
                *self = Self::Error {
                    maintenance: self.maintenance(),
                    e: SessionStateError::Mismatch {
                        expected: "Room",
                        actual: Box::new(mem::replace(self, Self::Init { maintenance: self.maintenance() })),
                    },
                    auto_retry: false,
                };
            },
            latest::ServerMessage::LoginSuccess => match self {
                Self::Lobby { login_state, .. } => { login_state.get_or_insert_with(LoginState::default); }
                Self::Room { login_state, .. } => { login_state.get_or_insert_with(LoginState::default); }
                Self::Error { .. } | Self::Init { .. } | Self::InitAutoRejoin { .. } | Self::Closed { .. } => *self = Self::Error {
                    maintenance: self.maintenance(),
                    e: SessionStateError::Mismatch {
                        expected: "Lobby or Room",
                        actual: Box::new(mem::replace(self, Self::Init { maintenance: self.maintenance() })),
                    },
                    auto_retry: false,
                },
            },
            latest::ServerMessage::MaintenanceNotice { start, duration } => match self {
                | Self::Error { maintenance, .. }
                | Self::Init { maintenance, .. }
                | Self::InitAutoRejoin { maintenance, .. }
                | Self::Lobby { maintenance, .. }
                | Self::Room { maintenance, .. }
                | Self::Closed { maintenance, .. }
                    => *maintenance = Some((start, duration)),
            },
        }
    }
}

impl<E> Default for SessionState<E> {
    fn default() -> Self {
        Self::Init {
            maintenance: None,
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

/// BizHawk for Linux comes with misconfigured file permissions.
/// See <https://aur.archlinux.org/cgit/aur.git/tree/PKGBUILD?h=bizhawk-monort>
#[cfg(target_os = "linux")]
pub fn fix_bizhawk_permissions(path: PathBuf) -> Pin<Box<dyn Future<Output = wheel::Result> + Send>> {
    Box::pin(async move {
        let metadata = fs::metadata(&path).await?;
        if metadata.is_dir() {
            fs::set_permissions(&path, fs::Permissions::from_mode(0o775)).await?;
            fs::read_dir(path)
                .try_for_each_concurrent(None, |entry| fix_bizhawk_permissions(entry.path())).await?;
        } else if metadata.is_file() {
            fs::set_permissions(&path, fs::Permissions::from_mode(if path.extension().is_some_and(|extension| extension == "sh") { 0o774 } else { 0o664 })).await?;
        }
        Ok(())
    })
}
