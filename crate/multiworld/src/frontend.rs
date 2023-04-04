use {
    std::num::NonZeroU8,
    async_proto::Protocol,
    ootr_utils::spoiler::HashIcon,
    crate::Filename,
};

pub const PORT: u16 = 24818;
pub const PROTOCOL_VERSION: u8 = 3; //TODO sync with JS code

#[derive(Debug, Protocol)]
pub enum ServerMessage {
    ItemQueue(Vec<u16>),
    GetItem(u16),
    PlayerName(NonZeroU8, Filename),
}

#[derive(Debug, Clone, Protocol)]
pub enum ClientMessage {
    PlayerId(NonZeroU8),
    PlayerName(Filename),
    SendItem {
        key: u32,
        kind: u16,
        target_world: NonZeroU8,
    },
    SaveData([u8; oottracker::save::SIZE]),
    FileHash([HashIcon; 5]),
    ResetPlayerId,
}
