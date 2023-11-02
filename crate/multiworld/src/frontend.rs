use {
    std::num::NonZeroU8,
    async_proto::Protocol,
    ootr_utils::spoiler::HashIcon,
    crate::Filename,
};

/// The default port used for connections between client and frontend.
/// If possible, prefer having the OS select an unused port dynamically, to allow multiple instances of multiworld to run in parallel on the same computer.
pub const PORT: u16 = 24818;
pub const PROTOCOL_VERSION: u8 = 4; //TODO sync with JS code

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
