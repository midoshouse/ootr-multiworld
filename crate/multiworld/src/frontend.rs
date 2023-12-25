use {
    std::{
        env,
        fmt,
        num::NonZeroU8,
    },
    async_proto::Protocol,
    enum_iterator::Sequence,
    ootr_utils::spoiler::HashIcon,
    serde::{
        Deserialize,
        Serialize,
    },
    crate::Filename,
};

/// The default port used for connections between client and frontend.
///
/// If possible, prefer having the OS select an unused port dynamically, to allow multiple instances of multiworld to run in parallel on the same computer.
pub const PORT: u16 = 24818;
pub const PROTOCOL_VERSION: u8 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Sequence, Deserialize, Serialize, clap::ValueEnum)]
#[clap(rename_all = "lower")]
pub enum Kind {
    Dummy,
    EverDrive,
    BizHawk,
    Pj64V3,
    Pj64V4,
}

impl Kind {
    pub fn is_supported(&self) -> bool {
        match self {
            Self::Dummy => false,
            Self::EverDrive => env::var_os("MHMW_EVERDRIVE").is_some_and(|envar| !envar.is_empty()), //TODO finish implementing, then enable by default
            Self::BizHawk => cfg!(any(target_os = "linux", target_os = "windows")),
            Self::Pj64V3 => cfg!(target_os = "windows"),
            Self::Pj64V4 => false, // hide until Project64 version 4 is released
        }
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dummy => write!(f, "(no frontend)"),
            Self::EverDrive => write!(f, "EverDrive"),
            Self::BizHawk => write!(f, "BizHawk"),
            Self::Pj64V3 | Self::Pj64V4 => write!(f, "Project64"),
        }
    }
}

#[derive(Debug, Protocol)]
pub enum ServerMessage {
    ItemQueue(Vec<u16>),
    GetItem(u16),
    PlayerName(NonZeroU8, Filename),
    ProgressiveItems(NonZeroU8, u32),
}

#[derive(Debug, Clone, Protocol)]
pub enum ClientMessage {
    PlayerId(NonZeroU8),
    PlayerName(Filename),
    SendItem {
        key: u64,
        kind: u16,
        target_world: NonZeroU8,
    },
    SaveData([u8; oottracker::save::SIZE]),
    FileHash([HashIcon; 5]),
    ResetPlayerId,
}
