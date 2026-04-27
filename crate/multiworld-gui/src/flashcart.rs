use {
    crate::{
        FrontendWriter,
        Message,
        UsbSerialPort,
    }, arrayref::{
        array_mut_ref,
        array_ref
    }, chrono::prelude::*, enum_iterator::all, futures::{
        TryStreamExt as _,
        stream::{
            self,
            Stream,
            StreamExt as _,
        },
    }, iced::{advanced::subscription::{
        EventStream,
        Recipe,
    }}, log_lock::lock, multiworld::{
        Filename, HintArea, OptHintArea, frontend::{
            ClientMessage,
            ServerMessage
        }
    }, n64flashcart::{
        self,
        DeviceError,
        ProtocolVer,
        USBDataType,
    }, num_traits::FromPrimitive as _, ootr_utils::spoiler::HashIcon, std::{
        any::TypeId,
        collections::{HashMap, VecDeque},
        hash::Hash as _,
        io::prelude::*,
        num::NonZeroU8,
        pin::Pin,
        sync::Arc,
        time::{
            Duration,
            Instant,
        },
    }, tokio::{
        select,
        sync::{
            Mutex,
            mpsc
        },
        task::spawn_blocking,
        time::{
            sleep,
            timeout
        }
    }
};
#[cfg(unix)] use std::ffi::OsString;

const DEBUG_LOGGING: bool = true;

macro_rules! log_println {
    ($enable_log:expr, $($arg:tt)*) => {
        if $enable_log {
            let timestamp = Utc::now().format("%Y-%m-%d %H:%M:%S");
            println!("[{timestamp}] {}", format_args!($($arg)*));
            let _ = lock!(log = crate::LOG; writeln!(&*log, "[{timestamp}] {}", format_args!($($arg)*)));
        }
    };
}

macro_rules! dbg_println {
    ($($arg:tt)*) => {
        log_println!(DEBUG_LOGGING, $($arg)*)
    };
}

const PROTOCOL_VERSION: u8 = 3;
const MW_SEND_OWN_ITEMS: u8 = 1;
const MW_PROGRESSIVE_ITEMS_ENABLE: u8 = 1;

#[derive(Debug, thiserror::Error)]
pub(crate) enum ConnectError {
    #[error("unknown branch identifier: 0x{0:02x}")]
    Branch(u8),
    #[error("failed to decode hash icon")]
    HashIcon,
    #[cfg(unix)]
    #[error("non-UTF-8 string: {}", .0.to_string_lossy())]
    OsString(OsString),
    #[error("received item for world 0")]
    PlayerId,
    #[error("unexpected handshake reply header: {0:x?}")]
    UnknownReplyHeader(USBDataType),
    #[error("unexpected handshake reply: {0:x?}")]
    UnknownReply([u8; 4]),
    #[error("unexpected handshake reply length: {0:x?}")]
    UnknownReplyLength(usize),
    #[error("failed to reply to handshake")]
    FailedReply,
    #[error("unhandled device error: {0:x?}")]
    DeviceError(DeviceError),
    #[error("received unknown message {0} from flashcart")]
    UnknownCommand(u8),
}

#[cfg(unix)]
impl From<OsString> for ConnectError {
    fn from(s: OsString) -> Self {
        Self::OsString(s)
    }
}

pub(crate) struct Subscription {
    pub(crate) log: bool,
    pub(crate) device: Option<UsbSerialPort>,
}

#[derive(Debug, Clone)]
pub(crate) enum InGameState {
    NotKnown,
    Desynced,
    FileSelect,
    InGame {
        internal_count: u16,
        item_pending: bool,
    },
}

#[derive(Debug)]
pub(crate) struct InGameStruct {
    _version: ootr_utils::Version,
    rx: mpsc::Receiver<ServerMessage>,
    item_queue: Vec<u16>,
    ingame_state: InGameState,
    filename: Option<Filename>,
    player_data: HashMap<NonZeroU8, (Filename, u32)>,
    message_queue: VecDeque<(USBDataType, Vec<u8>)>,
    pending_message: Option<Instant>,
} 

#[derive(Debug, Clone)]
pub(crate) struct FlashcartCache {
    player_data: HashMap<NonZeroU8, (Filename, u32)>,
}

#[derive(Debug, Clone)]
pub(crate) enum CommState {
    Disconnect,
    SendHandshake,
    WaitForGame,
    Handshake,
    Ready(Arc<Mutex<InGameStruct>>)
}

#[derive(Debug, Clone)]
pub enum FlashcartState {
    INITIALIZE,
    DISCONNECTED(FlashcartCache),
    SEARCHING(FlashcartCache),
    OPENING(String, FlashcartCache),
    CONNECTED(String, CommState, FlashcartCache, Arc<FlashcartGuard>)
}

#[derive(Debug)]
pub struct FlashcartGuard;

impl Drop for FlashcartGuard {
    fn drop(&mut self) {
        if n64flashcart::isopen() {
            n64flashcart::close();
        }
    }
}

#[derive(Debug)]
struct HandshakeResponse {
    version: ootr_utils::Version,
    player_id: NonZeroU8,
    file_hash: [HashIcon; 5],
}

async fn n64_recv() -> Result<(n64flashcart::Header, Vec<u8>), DeviceError> {
    loop {
        let res = spawn_blocking(|| n64flashcart::read()).await.expect("flashcart read panic");
        match res {
            Ok((header, data)) if header.datatype != USBDataType::EMPTY => {
                match header.datatype {
                    USBDataType::INGAME_STATE => dbg_println!("< RECV < Datatype: {:?}, Length: {}", header.datatype, header.length),
                    USBDataType::HEARTBEAT => dbg_println!("< RECV < Heartbeat"),
                    _ => dbg_println!("< RECV < Datatype: {:?}, Length: {}, data: {:?}", header.datatype, header.length, data)
                };
                return Ok((header, data));
            }
            Ok(_) => {
                sleep(Duration::from_millis(1)).await;
            }
            Err(e) => {
                // Purge read buffer to hopefully prevent errors on future reads
                match e {
                    DeviceError::READFAIL |
                    DeviceError::BADHEADER |
                    DeviceError::BADPADDING |
                    DeviceError::BADPACKSIZE |
                    DeviceError::_64D_BADCMP |
                    DeviceError::_64D_BADDMA |
                    DeviceError::SC64_COMMFAIL => {
                        spawn_blocking(|| n64flashcart::purge()).await.expect("flashcart purge panic");
                        n64_send(USBDataType::UNRECOVERABLE, vec![0u8; 16]).await?;
                    }
                    _ => {}
                }
                return Err(e);
            }
        };
    }
}

async fn n64_send(datatype: USBDataType, msg: Vec<u8>) -> Result<(), DeviceError> {
    dbg_println!("> SEND > Datatype: {:?}, Length: {}, data: {:?}", datatype, msg.len(), msg);
    let header = n64flashcart::Header { datatype: datatype, length: msg.len() };
    let status = spawn_blocking(|| n64flashcart::write(header, msg)).await.expect("flashcart send panic");
    match status {
        DeviceError::OK => Ok(()),
        _ => Err(status)
    }
}

async fn send_handshake() {
    let _ = n64_send(USBDataType::HANDSHAKE, "cmdt".as_bytes().to_vec()).await;
}

async fn send_handshake_response() -> Result<(), DeviceError> {
    let mut msg = "MW".as_bytes().to_vec();
    msg.push(PROTOCOL_VERSION);
    msg.push(MW_SEND_OWN_ITEMS);
    msg.push(MW_PROGRESSIVE_ITEMS_ENABLE);

    n64_send(USBDataType::HANDSHAKE, msg).await
}

async fn send_reset() {
    let _ = n64_send(USBDataType::RESET, vec![0u8, 16]).await;
}

async fn send_ack() {
    let _ = n64_send(USBDataType::ACK_MESSAGE, vec![0u8, 16]).await;
}

async fn send_err() {
    let _ = n64_send(USBDataType::UNRECOVERABLE, vec![0u8, 16]).await;
}

async fn send_player_data(world: NonZeroU8, name: Filename, progressive_items: u32) -> (USBDataType, Vec<u8>) {
    let mut buf = [0; 16];

    buf[0] = world.get();
    *array_mut_ref![buf, 1, 8] = name.0;
    *array_mut_ref![buf, 9, 4] = progressive_items.to_be_bytes();
    
    return (USBDataType::PLAYER_NAMES, Vec::from(buf));
}

async fn get_item(queue: &[u16], internal_count: &u16, messages: &mut VecDeque<(USBDataType, Vec<u8>)>) -> bool {
    if let Some(item) = queue.get(usize::from(*internal_count)) {
        messages.push_back(send_item(*item).await);
        return true;
    } else {
        return false;
    }
}

async fn send_item(item: u16) -> (USBDataType, Vec<u8>) {
    let mut msg: Vec<u8> = Vec::new();

    let [b1, b2] = item.to_be_bytes();
    msg.push(b1);
    msg.push(b2);

    return (USBDataType::SEND_ITEM, msg);
}

async fn process_n64_packet(header: n64flashcart::Header, data: Vec<u8>, struc: &mut InGameStruct) -> Result<(Option<InGameState>, Option<Vec<Message>>), DeviceError>
{
    match header.datatype {
        USBDataType::HANDSHAKE | USBDataType::RESET => {
            Ok((Some(InGameState::NotKnown), None))
        },

        // Summercart menu responses are UNFloader-compatible.
        // Everdrive gets filtered out by n64flashcart.
        USBDataType::TEXT => {
            if data.len() >= 6 {
                let data_slice = data.into_boxed_slice();
                let text = *array_ref![data_slice, 0, 6];
                match text {
                    [b'j', b'o', b'y', b'b', b'u', b's'] => Ok((Some(InGameState::NotKnown), None)),
                    _ => Err(DeviceError::SC64_FIRMWAREUNSUPPORTED),
                }
            } else {
                Err(DeviceError::SC64_FIRMWAREUNSUPPORTED)
            }
        },

        USBDataType::SAVE_FILENAME => {
            if data.len() >= 8 {
                let data_slice = data.into_boxed_slice();
                let filename = Filename(*array_ref![data_slice, 0, 8]);
                struc.filename = Some(filename);

                send_ack().await;
                Ok((Some(InGameState::FileSelect), Some(vec![Message::Plugin(Box::new(ClientMessage::PlayerName(filename)))])))
            } else {
                send_err().await;
                Ok((None, None))
            }
        },

        USBDataType::INGAME_STATE => {
            if let Ok(savedata) = TryInto::<[u8 ; 5200]>::try_into(data) {
                let mut messages = Vec::new();
                
                let filename = Filename(*array_ref![savedata, 0x024, 8]);
                if Some(filename) != struc.filename {
                    struc.filename = Some(filename);
                    messages.push(Message::Plugin(Box::new(ClientMessage::PlayerName(filename))));
                }
                let internal_count = u16::from_be_bytes(*array_ref![savedata, 0x90, 2]);

                messages.push(Message::Plugin(Box::new(ClientMessage::SaveData(savedata))));
                let item_pending = if let InGameState::InGame { item_pending, .. } = struc.ingame_state {
                    item_pending
                } else {
                    for (world, (name, progressive_items)) in struc.player_data.clone() {
                        struc.message_queue.push_back(send_player_data(world, name, progressive_items).await);
                    }
                    get_item(&struc.item_queue, &internal_count, &mut struc.message_queue).await
                };

                send_ack().await;
                Ok((Some(InGameState::InGame { internal_count, item_pending }), Some(messages)))
            } else {
                send_err().await;
                Ok((Some(InGameState::NotKnown), None))
            }
        },

        USBDataType::SEND_ITEM => {
            if data.len() >= 11 {
                let data_slice = data.into_boxed_slice();

                let kind = u16::from_be_bytes(*array_ref![data_slice, 8, 2]);
                let target_world = NonZeroU8::new(data_slice[10]).ok_or(ConnectError::PlayerId).unwrap();

                dbg_println!("Got item {} for world {}", kind, target_world);

                let message = Message::Plugin(Box::new(ClientMessage::SendItem {
                    key: u64::from_be_bytes(*array_ref![data_slice, 0, 8]),
                    kind: kind,
                    target_world: target_world,
                }));

                send_ack().await;
                Ok((None, Some(vec![message])))
            } else {
                send_err().await;
                Ok((None, None))
            }
        },

        USBDataType::ACK_MESSAGE => {
            let InGameStruct {
                ref mut message_queue,
                ref mut pending_message,
                ..
            } = *struc;
            // Check if we actually sent a message to acknowledge.
            if let None = pending_message {
                Ok((Some(InGameState::Desynced), None))
            } else {
                let _ = message_queue.pop_front();
                *pending_message = None;
                Ok((None, None))
            }
        },

        USBDataType::ITEM_GIVEN => {
            let InGameStruct {
                ref mut ingame_state,
                ref mut message_queue,
                ..
            } = *struc;
            if let InGameState::InGame { ref mut internal_count, ref mut item_pending } = ingame_state {
                if !*item_pending {
                    Ok((Some(InGameState::Desynced), None))
                } else {
                    //send_ack().await;
                    *internal_count += 1;
                    if !get_item(&struc.item_queue, internal_count, message_queue).await {
                        *item_pending = false;
                    }
                    Ok((None, None))
                }
            } else {
                Err(DeviceError::READFAIL)
            }
        }

        USBDataType::UNRECOVERABLE => {
            // Check if we actually sent a message to acknowledge.
            if let None = struc.pending_message {
                Ok((Some(InGameState::Desynced), None))
            } else {
                // Retry sending message, don't remove from queue yet
                struc.pending_message = None;
                Ok((None, None))
            }
        },

        USBDataType::DUNGEON_REWARDS => {
            if let Ok(rewarddata) = TryInto::<[u8 ; 19]>::try_into(data) {
                let [
                    _,
                    emerald_world,
                    emerald_area,
                    ruby_world,
                    ruby_area,
                    sapphire_world,
                    sapphire_area,
                    light_world,
                    light_area,
                    forest_world,
                    forest_area,
                    fire_world,
                    fire_area,
                    water_world,
                    water_area,
                    shadow_world,
                    shadow_area,
                    spirit_world,
                    spirit_area
                ] = rewarddata;
                let message = vec![Message::Plugin(Box::new(ClientMessage::DungeonRewardInfo {
                    emerald: if let (Some(world), Some(area)) = (NonZeroU8::new(emerald_world), OptHintArea::from_u8(emerald_area).and_then(|area| HintArea::try_from(area).ok())) { Some((world, area)) } else { None },
                    ruby: if let (Some(world), Some(area)) = (NonZeroU8::new(ruby_world), OptHintArea::from_u8(ruby_area).and_then(|area| HintArea::try_from(area).ok())) { Some((world, area)) } else { None },
                    sapphire: if let (Some(world), Some(area)) = (NonZeroU8::new(sapphire_world), OptHintArea::from_u8(sapphire_area).and_then(|area| HintArea::try_from(area).ok())) { Some((world, area)) } else { None },
                    light: if let (Some(world), Some(area)) = (NonZeroU8::new(light_world), OptHintArea::from_u8(light_area).and_then(|area| HintArea::try_from(area).ok())) { Some((world, area)) } else { None },
                    forest: if let (Some(world), Some(area)) = (NonZeroU8::new(forest_world), OptHintArea::from_u8(forest_area).and_then(|area| HintArea::try_from(area).ok())) { Some((world, area)) } else { None },
                    fire: if let (Some(world), Some(area)) = (NonZeroU8::new(fire_world), OptHintArea::from_u8(fire_area).and_then(|area| HintArea::try_from(area).ok())) { Some((world, area)) } else { None },
                    water: if let (Some(world), Some(area)) = (NonZeroU8::new(water_world), OptHintArea::from_u8(water_area).and_then(|area| HintArea::try_from(area).ok())) { Some((world, area)) } else { None },
                    shadow: if let (Some(world), Some(area)) = (NonZeroU8::new(shadow_world), OptHintArea::from_u8(shadow_area).and_then(|area| HintArea::try_from(area).ok())) { Some((world, area)) } else { None },
                    spirit: if let (Some(world), Some(area)) = (NonZeroU8::new(spirit_world), OptHintArea::from_u8(spirit_area).and_then(|area| HintArea::try_from(area).ok())) { Some((world, area)) } else { None },
                }))];

                send_ack().await;
                Ok((None, Some(message)))
            } else {
                send_err().await;
                Ok((None, None))
            }
        }

        USBDataType::HEARTBEAT => Ok((None, None)),

        USBDataType::RAWBINARY => {
            // Read arbitrary memory from console.
            // Currently unused, but skeleton kept to
            // keep message queue moving if added later.

            let InGameStruct {
                ref mut message_queue,
                ref mut pending_message,
                ..
            } = *struc;
            // Check if we actually sent a message to acknowledge.
            if let None = pending_message {
                Ok((Some(InGameState::Desynced), None))
            } else {
                if let Some(_) = message_queue.pop_front() {
                    // Handle raw byte message here
                    *pending_message = None;
                }
                send_ack().await;
                Ok((None, None))
            }
        }

        _ => {
            Err(DeviceError::SC64_FIRMWAREUNSUPPORTED)
        }
    }
}

async fn process_handshake(header: n64flashcart::Header, data: Vec<u8>) -> Result<HandshakeResponse, ConnectError> {
    if header.datatype != USBDataType::HANDSHAKE {
        return Err(ConnectError::UnknownReplyHeader(header.datatype));
    }
    let data_size = data.len();
    match TryInto::<[u8 ; 16]>::try_into(data) {
        Ok(value) => {
            match value {
                [b'O', b'o', b'T', b'R', PROTOCOL_VERSION, major, minor, patch, branch, supplementary, player_id, hash1, hash2, hash3, hash4, hash5] => {
                    dbg_println!("Handshake reply received. Repeating protocol version to finalize handshake");
                    match send_handshake_response().await {
                        Ok(_) => {
                            dbg_println!("Protocol version sent");

                            let version = ootr_utils::Version::from_bytes([major, minor, patch, branch, supplementary]).ok_or_else(|| ConnectError::Branch(branch))?;
                            let player_id = NonZeroU8::new(player_id).ok_or(ConnectError::PlayerId)?;
                            let file_hash: [HashIcon; 5] = [
                                all().nth(hash1.into()).ok_or(ConnectError::HashIcon)?,
                                all().nth(hash2.into()).ok_or(ConnectError::HashIcon)?,
                                all().nth(hash3.into()).ok_or(ConnectError::HashIcon)?,
                                all().nth(hash4.into()).ok_or(ConnectError::HashIcon)?,
                                all().nth(hash5.into()).ok_or(ConnectError::HashIcon)?,
                            ];
                            
                            Ok(HandshakeResponse { version: version, player_id, file_hash })
                        },
                        Err(_) => {
                            dbg_println!("Failed to send protocol version, restarting handshake");
                            Err(ConnectError::FailedReply)
                        }
                    }
                },
                _ => {
                    dbg_println!("Invalid handshake reply, restarting handshake");
                    Err(ConnectError::UnknownReply(value[0..4].try_into().unwrap()))
                }
            } 
        },
        Err(_) => {
            dbg_println!("Invalid handshake length, restarting handshake");
            Err(ConnectError::UnknownReplyLength(data_size))
        }
    }
}

async fn read(comm_state: &CommState, cache: &FlashcartCache) -> Result<(Option<CommState>, Vec<Message>), DeviceError> {
    let mut messages = Vec::new();
    let next_state = match comm_state {
        CommState::Disconnect => Some(CommState::Disconnect),
        CommState::WaitForGame => {
            match timeout(Duration::from_secs(10), n64_recv()).await {
                Ok(read_result) => {
                    match read_result {
                        Ok((header, _data)) => {
                            match header.datatype {
                                USBDataType::HANDSHAKE | USBDataType::RESET => {
                                    Some(CommState::SendHandshake)
                                },
                                _ => {
                                    send_reset().await;
                                    None
                                }
                            }
                        }
                        Err(e) => {
                            dbg_println!("Read error while waiting for handshake, {}", e.value());
                            Some(CommState::Disconnect)
                        }
                    }
                },
                Err(_) => {
                    dbg_println!("No message from N64 in 10 seconds");
                    Some(CommState::WaitForGame)
                }
            }
        },
        CommState::SendHandshake => {
            send_handshake().await;
            Some(CommState::Handshake)
        },
        CommState::Handshake => {
            match timeout(Duration::from_secs(10), n64_recv()).await {
                Ok(read_result) => {
                    match read_result {
                        Ok((header, data)) => {
                            let mut response = None;
                            let mut errors = Vec::default();
                            match process_handshake(header, data).await {
                                Ok(resp) => response = Some(resp),
                                Err(e) => errors.push(e),
                            }
                            if let Some(HandshakeResponse { version, player_id, file_hash }) = response {
                                let (tx, rx) = mpsc::channel(1_024);

                                messages.push(Message::FrontendConnected(FrontendWriter::Mpsc(tx)));
                                messages.push(Message::Plugin(Box::new(ClientMessage::PlayerId(player_id))));
                                messages.push(Message::Plugin(Box::new(ClientMessage::FileHash(Some(file_hash)))));
                                messages.push(Message::FlashcartHandshakeSuccessful());

                                let struc = InGameStruct {
                                    _version: version,
                                    rx: rx,
                                    item_queue: Vec::default(),
                                    ingame_state: InGameState::NotKnown,
                                    filename: None,
                                    player_data: cache.player_data.clone(),
                                    message_queue: VecDeque::default(),
                                    pending_message: None,
                                };

                                Some(CommState::Ready(Arc::new(Mutex::new(struc))))
                            } else if errors.is_empty() {
                                None
                            } else {
                                messages.push(Message::FlashcartHandshakeFailed(Arc::new(errors)));
                                Some(CommState::WaitForGame)
                            }
                        }
                        Err(e) => {
                            dbg_println!("Read error while finalizing for handshake, {}", e.value());
                            Some(CommState::Disconnect)
                        }
                    }
                },
                Err(_) => {
                    dbg_println!("No message from N64 in 10 seconds");
                    Some(CommState::Disconnect)
                }
            }
        },
        CommState::Ready(_struc) => {
            let mut struc = _struc.lock().await;

            dbg_println!("InGameState: {:?}", struc.ingame_state);

            // Handle potentially lost acknowledge packet without
            // total communications loss. Force reset the connection
            // to avoid duplicating items and desyncing the item counter.
            // 5 seconds is approximately the time between heartbeats,
            // with 10 seconds as the normal timeout. 7 seconds is checked
            // here as the ack should not take longer than a full heartbeat
            // cycle to be sent, accounting for some generous console lag.
            // Timeout starts after the message is sent to minimize lag.
            if let Some(message_timeout) = struc.pending_message {
                if message_timeout.elapsed().as_secs_f32() > 7.0 {
                    dbg_println!("N64 did not acknowledge message, resetting");
                    return Ok((Some(CommState::Disconnect), messages));
                }
            }

            if let None = struc.pending_message {
                if let Some((datatype, msg)) = struc.message_queue.front() {
                    match n64_send(*datatype, msg.clone()).await {
                        Ok(_) => struc.pending_message = Some(Instant::now()),
                        Err(_) => struc.pending_message = None,
                    }
                }
            }

            select! {
                read_or_timeout = timeout(Duration::from_secs(10), n64_recv()) => {
                    match read_or_timeout {
                        Ok(read_result) => {
                            match read_result {
                                Ok((header, data)) => {
                                    let datatype = header.datatype.value();
                                    match process_n64_packet(header, data, &mut struc).await {
                                        Ok((state_, messages_)) => {
                                            if let Some(msg) = messages_ {
                                                messages.extend(msg);
                                            }
                                            if let Some(InGameState::NotKnown) = state_ {
                                                Some(CommState::WaitForGame)
                                            } else if let Some(InGameState::Desynced) = state_ {
                                                Some(CommState::Disconnect)
                                            } else if let Some(value) = state_ {
                                                struc.ingame_state = value;
                                                None
                                            } else {
                                                None
                                            }
                                        }
                                        Err(e) => {
                                            let mut errors = Vec::default();
                                            if let DeviceError::SC64_FIRMWAREUNSUPPORTED = e {
                                                errors.push(ConnectError::UnknownCommand(datatype));
                                            } else {
                                                errors.push(ConnectError::DeviceError(e));
                                            }
                                            dbg_println!("Error processing data from N64, {:?}", e);
                                            messages.push(Message::FlashcartCommError(Arc::new(errors)));
                                            None
                                        }
                                    }
                                },
                                Err(e) => {
                                    dbg_println!("Error receiving from N64, {:?}", e);
                                    let errors = vec![ConnectError::DeviceError(e)];
                                    messages.push(Message::FlashcartCommError(Arc::new(errors)));
                                    Some(CommState::Disconnect)
                                }
                            }
                        },
                        Err(_) => {
                            dbg_println!("No message from N64 in 10 seconds");
                            Some(CommState::Disconnect)
                        }
                    }
                },
                Some(msg) = struc.rx.recv() => {
                    dbg_println!("Received message from MH, {:?}", msg);
                    match msg {
                        ServerMessage::ItemQueue(items) => {
                            struc.item_queue = items;
                            let InGameStruct {
                                ref mut ingame_state,
                                ref mut message_queue,
                                ref item_queue,
                                ..
                            } = *struc;
                            if let InGameState::InGame { ref internal_count, ref mut item_pending } = ingame_state {
                                if !*item_pending {
                                    if let true = get_item(item_queue, internal_count, message_queue).await {
                                        *item_pending = true;
                                    }
                                }
                            }
                        },
                        ServerMessage::GetItem(item) => {
                            struc.item_queue.push(item);
                            let InGameStruct {
                                ref mut ingame_state,
                                ref mut message_queue,
                                ref item_queue,
                                ..
                            } = *struc;
                            if let InGameState::InGame { ref internal_count, ref mut item_pending } = ingame_state {
                                if !*item_pending {
                                    if let true = get_item(item_queue, internal_count, message_queue).await {
                                        *item_pending = true;
                                    }
                                }
                            }
                        },
                        ServerMessage::PlayerName(world, new_name) => {
                            let InGameStruct {
                                ref mut message_queue,
                                ref mut player_data,
                                ..
                            } = *struc;
                            let (name, progressive_items) = player_data.entry(world).or_default();
                            *name = new_name;
                            message_queue.push_back(send_player_data(world, *name, *progressive_items).await);
                        },
                        ServerMessage::ProgressiveItems(world, new_progressive_items) => {
                            let InGameStruct {
                                ref mut message_queue,
                                ref mut player_data,
                                ..
                            } = *struc;
                            let (name, progressive_items) = player_data.entry(world).or_default();
                            *progressive_items = new_progressive_items;
                            message_queue.push_back(send_player_data(world, *name, *progressive_items).await);
                        },
                    }
                    None
                }
            }
        }
    };

    Ok((next_state, messages))
}

impl Recipe for Subscription {
    type Output = Message;

    fn hash(&self, state: &mut iced::advanced::subscription::Hasher) {
        TypeId::of::<Self>().hash(state);

        if let Some(device) = &self.device {
            device.vid.hash(state);
            device.pid.hash(state);
            device.serial.hash(state); 
        } else {
            0.hash(state);
        }
    }

    fn stream(self: Box<Self>, _: EventStream) -> Pin<Box<dyn Stream<Item = Message> + Send>> {
        let log = self.log || DEBUG_LOGGING;
        stream::try_unfold(FlashcartState::INITIALIZE, move |state| {
            let device = self.device.clone();
            async move {
            let _ = sleep(Duration::from_millis(1)).await;
            let mut messages: Vec<Message> = Vec::new();

            let new_state = match &state {
                FlashcartState::INITIALIZE => {
                    n64flashcart::initialize();
                    n64flashcart::set_protocol(ProtocolVer::VERSION2);
                    Some(FlashcartState::SEARCHING(FlashcartCache { player_data: HashMap::default() }))
                },
                FlashcartState::DISCONNECTED(cache) => {
                    log_println!(log, "Flashcart: waiting 5 seconds before next scan");
                    let _ = sleep(Duration::from_secs(5)).await;
                    Some(FlashcartState::SEARCHING(cache.to_owned()))
                },
                FlashcartState::SEARCHING(cache) => {
                    if let Some(attached_device) = device {
                        let status = n64flashcart::connect(attached_device.vid, attached_device.pid, &attached_device.serial);
                        if status == DeviceError::CARTFINDFAIL {
                            n64flashcart::initialize();
                            n64flashcart::set_protocol(ProtocolVer::VERSION2);
                            Some(FlashcartState::DISCONNECTED(cache.to_owned()))
                        } else if status != DeviceError::OK {
                            Some(FlashcartState::DISCONNECTED(cache.to_owned()))
                        } else {
                            let cart_name = n64flashcart::cart_type_to_str(n64flashcart::get_cart());
                            Some(FlashcartState::OPENING(cart_name.to_string(), cache.to_owned()))
                        }
                    } else {
                        None
                    }
                },
                FlashcartState::OPENING(name, cache) => {
                    if let Some(_) = device {
                        let status = n64flashcart::open();
                        if status != DeviceError::OK {
                            dbg_println!("Failed to open USB connection to flashcart, retrying, error code {}", status.value());
                            if status == DeviceError::CANTOPEN {
                                messages.push(Message::FlashcartLocked);
                            }
                            Some(FlashcartState::DISCONNECTED(cache.to_owned()))
                        } else {
                            dbg_println!("Flashcart USB connection opened");
                            Some(FlashcartState::CONNECTED(name.to_owned(), CommState::WaitForGame, cache.to_owned(), Arc::new(FlashcartGuard)))
                        }
                    } else {
                        Some(FlashcartState::SEARCHING(cache.to_owned()))
                    }
                },
                FlashcartState::CONNECTED(name, comm_state, cache, guard) => {
                    if let Some(_) = device {
                        match read(comm_state, cache).await {
                            Ok((next_state, m)) => {
                                messages.extend(m);
                                match comm_state {
                                    CommState::Ready(_struc) => {
                                        match next_state {
                                            Some(ready_state @ CommState::Ready(_)) => Some(FlashcartState::CONNECTED(name.to_owned(), ready_state, cache.to_owned(), guard.to_owned())),
                                            Some(CommState::Disconnect) => {
                                                let struc = _struc.lock().await;
                                                let mut new_cache = cache.to_owned();
                                                new_cache.player_data = struc.player_data.clone();
                                                Some(FlashcartState::DISCONNECTED(new_cache))
                                            },
                                            Some(new_state) => {
                                                let struc = _struc.lock().await;
                                                let mut new_cache = cache.to_owned();
                                                new_cache.player_data = struc.player_data.clone();
                                                Some(FlashcartState::CONNECTED(name.to_owned(), new_state, new_cache, guard.to_owned()))
                                            },
                                            None => None,
                                        }
                                    },
                                    _ => {
                                        match next_state {
                                            Some(CommState::Disconnect) => Some(FlashcartState::DISCONNECTED(cache.to_owned())),
                                            Some(new_state) => Some(FlashcartState::CONNECTED(name.to_owned(), new_state, cache.to_owned(), guard.to_owned())),
                                            None => None,
                                        }
                                    }
                                }
                            },
                            Err(e) => {
                                messages.push(Message::FlashcartCommError(Arc::new(vec![ConnectError::DeviceError(e)])));
                                Some(FlashcartState::CONNECTED(name.to_owned(), comm_state.to_owned(), cache.to_owned(), guard.to_owned()))
                            }
                        }
                    } else {
                        Some(FlashcartState::SEARCHING(cache.to_owned()))
                    }
                }
            };

            if let Some(value) = &new_state {
                messages.push(Message::FlashcartStateChanged(value.clone()));
            }

            Ok::<_, ConnectError>(Some((stream::iter(messages).map(Ok::<_, ConnectError>), new_state.unwrap_or(state))))
        }}).try_flatten().then(|res| async move {
            let mut print_debug = DEBUG_LOGGING;

            if let Ok(message) = &res {
                if let Message::Plugin(plugin) = message {
                    if let ClientMessage::SaveData(_) = plugin.as_ref() {
                        print_debug = false;
                    }
                }
            }

            if print_debug {
                dbg_println!("{:?}", res);
            }
            res.unwrap_or_else(|e| Message::FrontendSubscriptionError(Arc::new(e.into())))
        }).boxed()
    }
}
