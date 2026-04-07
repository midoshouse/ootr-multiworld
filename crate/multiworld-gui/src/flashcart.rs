use {
    crate::{
        FrontendWriter,
        Message
    },
    arrayref::{
        array_mut_ref,
        array_ref
    },
    chrono::prelude::*,
    enum_iterator::all,
    futures::{
        TryStreamExt as _,
        stream::{
            self,
            Stream,
            StreamExt as _,
        },
    },
    iced::advanced::subscription::{
        EventStream,
        Recipe,
    },
    log_lock::lock,
    multiworld::{
        Filename,
        frontend::{
            ClientMessage,
            ServerMessage
        },
        HintArea,
        OptHintArea,
    },
    n64flashcart::{
        self,
        DeviceError,
        ProtocolVer,
        USBDataType,
    },
    num_traits::FromPrimitive as _,
    ootr_utils::spoiler::HashIcon,
    std::{
        any::TypeId,
        collections::HashMap,
        hash::Hash as _,
        io::prelude::*,
        mem,
        num::NonZeroU8,
        pin::Pin,
        sync::Arc,
        time::Duration
    },
    tokio::{
        select,
        sync::{
            Mutex,
            mpsc
        },
        time::{
            sleep,
            timeout
        }
    },
};
#[cfg(unix)] use std::ffi::OsString;

const DEBUG_LOGGING: bool = true;

macro_rules! dbg_println {
    ($($arg:tt)*) => {
        if DEBUG_LOGGING {
            println!($($arg)*);
        }
    };
}

const PROTOCOL_VERSION: u8 = 2;
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
    #[error("no reply from the console")]
    NoMessage,
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
}

#[derive(Debug, Clone)]
pub(crate) enum InGameState {
    NotKnown,
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
} 

#[derive(Debug, Clone)]
pub(crate) enum CommState {
    SendHandshake,
    WaitForGame,
    Handshake,
    Ready(Arc<Mutex<InGameStruct>>)
}

#[derive(Debug, Clone)]
pub enum FlashcartState {
    DISCONNECTED,
    SEARCHING,
    OPENING(String),
    CONNECTED(String, CommState)
}

#[derive(Debug)]
struct HandshakeResponse {
    version: ootr_utils::Version,
    player_id: NonZeroU8,
    file_hash: [HashIcon; 5],
}

async fn n64_recv() -> Result<(n64flashcart::Header, Vec<u8>), DeviceError> {
    loop {
        match n64flashcart::read() {
            Ok((header, data)) => {
                if header.datatype != USBDataType::EMPTY {
                    return Ok((header, data));
                }
            }
            Err(e) => return Err(e),
        };
    }
}

fn n64_send(datatype: USBDataType, msg: Vec<u8>) -> Result<(), DeviceError> {
    let header = n64flashcart::Header { datatype: datatype, length: msg.len() };
    let status = n64flashcart::write(header, msg);
    match status {
        DeviceError::OK => Ok(()),
        _ => Err(status)
    }
}

fn send_handshake() {
    let msg = "cmdt".as_bytes().to_vec();

    let _ = n64_send(USBDataType::HANDSHAKE, msg);
}

fn send_handshake_response() -> Result<(), DeviceError> {
    let mut msg = "MW".as_bytes().to_vec();
    msg.push(PROTOCOL_VERSION);
    msg.push(MW_SEND_OWN_ITEMS);
    msg.push(MW_PROGRESSIVE_ITEMS_ENABLE);

    n64_send(USBDataType::HANDSHAKE, msg)
}

fn send_reset() {
    let msg = "cmdt".as_bytes().to_vec();

    let _ = n64_send(USBDataType::RESET, msg);
}

fn send_player_data(world: NonZeroU8, name: Filename, progressive_items: u32) -> Result<(), DeviceError> {
    let mut buf = [0; 16];

    buf[0] = world.get();
    *array_mut_ref![buf, 1, 8] = name.0;
    *array_mut_ref![buf, 9, 4] = progressive_items.to_be_bytes();

    n64_send(USBDataType::PLAYER_NAMES, Vec::from(buf))
}

async fn get_item(queue: &[u16], internal_count: &mut u16) -> Result<bool, DeviceError> {
    if let Some(item) = queue.get(usize::from(*internal_count)) {
        match send_item(*item) {
            Ok(_) => {
                *internal_count += 1;
                Ok(true)
            },
            Err(_) => {
                Ok(false)
            }
        }
    } else {
        Ok(false)
    }
}

fn send_item(item: u16) -> Result<(), DeviceError> {
    let mut msg: Vec<u8> = Vec::new();

    let [b1, b2] = item.to_be_bytes();
    msg.push(b1);
    msg.push(b2);

    n64_send(USBDataType::SEND_ITEM, msg)
}

async fn process_n64_packet(header: n64flashcart::Header, data: Vec<u8>, struc: &mut InGameStruct) -> Result<(Option<InGameState>, Option<Vec<Message>>), DeviceError>
{
    match header.datatype {
        USBDataType::INGAME_STATE => dbg_println!("Datatype: {:?}, Length: {}", header.datatype, header.length),
        _ => dbg_println!("Datatype: {:?}, Length: {}, data: {:?}", header.datatype, header.length, data)
    };

    match header.datatype {
        USBDataType::HANDSHAKE | USBDataType::RESET => {
            Ok((Some(InGameState::NotKnown), None))
        },

        // Summercart menu responses are UNFloader-compatible.
        // Everdrive gets filtered out by n64flashcart.
        USBDataType::TEXT => {
            if let Ok(text) = TryInto::<[u8 ; 6]>::try_into(data) {
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

                Ok((Some(InGameState::FileSelect), Some(vec![Message::Plugin(Box::new(ClientMessage::PlayerName(filename)))])))
            } else {
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
                let mut internal_count = u16::from_be_bytes(*array_ref![savedata, 0x90, 2]);

                messages.push(Message::Plugin(Box::new(ClientMessage::SaveData(savedata))));
                let item_pending = if let InGameState::InGame { item_pending, .. } = struc.ingame_state {
                    item_pending
                } else {
                    for (world, (name, progressive_items)) in mem::take(&mut struc.player_data) {
                        let _ = send_player_data(world, name, progressive_items);
                    }
                    get_item(&struc.item_queue, &mut internal_count).await?
                };
                Ok((Some(InGameState::InGame { internal_count, item_pending }), Some(messages)))
            } else {
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

                Ok((None, Some(vec![message])))
            } else {
                Ok((None, None))
            }
        },

        USBDataType::ACK_ITEM => {
            if let InGameState::InGame { ref mut internal_count, ref mut item_pending } = struc.ingame_state {
                if !get_item(&struc.item_queue, internal_count).await? {
                    *item_pending = false;
                }
            }
            Ok((None, None))
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
                Ok((None, Some(message)))
            } else {
                Ok((None, None))
            }
        }

        USBDataType::HEARTBEAT => Ok((None, None)),
        USBDataType::EMPTY => Ok((None, None)),

        _ => {
            Err(DeviceError::SC64_FIRMWAREUNSUPPORTED)
        }
    }
}

fn process_handshake(header: n64flashcart::Header, data: Vec<u8>) -> Result<HandshakeResponse, ConnectError> {
    if header.datatype == USBDataType::EMPTY {
        return Err(ConnectError::NoMessage);
    }
    if header.datatype != USBDataType::HANDSHAKE {
        return Err(ConnectError::UnknownReplyHeader(header.datatype));
    }
    let data_size = data.len();
    match TryInto::<[u8 ; 16]>::try_into(data) {
        Ok(value) => {
            match value {
                [b'O', b'o', b'T', b'R', PROTOCOL_VERSION, major, minor, patch, branch, supplementary, player_id, hash1, hash2, hash3, hash4, hash5] => {
                    dbg_println!("Handshake reply received. Repeating protocol version to finalize handshake");
                    match send_handshake_response() {
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

async fn read(name: &String, comm_state: &CommState) -> Result<(Option<FlashcartState>, Vec<Message>), DeviceError> {
    let mut messages = Vec::new();
    let next_state = match comm_state {
        CommState::WaitForGame => {
            match n64flashcart::read() {
                Ok((header, _data)) => {
                    match header.datatype {
                        USBDataType::HANDSHAKE | USBDataType::RESET => {
                            Some(FlashcartState::CONNECTED(name.to_owned(), CommState::SendHandshake))
                        },
                        USBDataType::EMPTY => None,
                        _ => {
                            send_reset();
                            None
                        }
                    }
                }
                Err(e) => {
                    dbg_println!("Read error while waiting for handshake, {}", e.value());
                    Some(FlashcartState::DISCONNECTED)
                }
            }
        },
        CommState::SendHandshake => {
            send_handshake();
            Some(FlashcartState::CONNECTED(name.to_owned(), CommState::Handshake))
        },
        CommState::Handshake => {
            match n64flashcart::read() {
                Ok((header, data)) => {
                    let mut response = None;
                    let mut errors = Vec::default();
                    match process_handshake(header, data) {
                        Ok(resp) => {
                            response = Some(resp);
                        }
                        Err(e) => {
                            match e {
                                ConnectError::NoMessage => {}
                                _ => {
                                    errors.push((name.to_owned(), e));
                                }
                            }
                        }
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
                            player_data: HashMap::default()
                        };

                        Some(FlashcartState::CONNECTED(name.to_owned(), CommState::Ready(Arc::new(Mutex::new(struc)))))
                    } else if errors.is_empty() {
                        None
                    } else {
                        messages.push(Message::FlashcartHandshakeFailed(Arc::new(errors)));
                        Some(FlashcartState::CONNECTED(name.to_owned(), CommState::WaitForGame))
                    }
                }
                Err(e) => {
                    dbg_println!("Read error while finalizing for handshake, {}", e.value());
                    Some(FlashcartState::DISCONNECTED)
                }
            }
        },
        CommState::Ready(_struc) => {
            let mut struc = _struc.lock().await;

            dbg_println!("InGameState: {:?}", struc.ingame_state);

            select! {
                n64_or_timeout = timeout(Duration::from_secs(10), n64_recv()) => {
                    match n64_or_timeout {
                        Ok(n64_result) => {
                            match n64_result {
                                Ok((header, data)) => {
                                    let datatype = header.datatype.value();
                                    match process_n64_packet(header, data, &mut struc).await {
                                        Ok((state_, messages_)) => {
                                            if let Some(msg) = messages_ {
                                                messages.extend(msg);
                                            }
                                            if let Some(InGameState::NotKnown) = state_ {
                                                Some(FlashcartState::CONNECTED(name.to_owned(), CommState::WaitForGame))
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
                                                errors.push((name.to_owned(), ConnectError::UnknownCommand(datatype)));
                                            } else {
                                                errors.push((name.to_owned(), ConnectError::DeviceError(e)));
                                            }
                                            dbg_println!("Error processing data from N64, {:?}", e);
                                            messages.push(Message::FlashcartCommError(Arc::new(errors)));
                                            None
                                        }
                                    }
                                },
                                Err(e) => {
                                    dbg_println!("Error receiving from N64, {:?}", e);
                                    let errors = vec![(name.to_owned(), ConnectError::DeviceError(e))];
                                    messages.push(Message::FlashcartCommError(Arc::new(errors)));
                                    Some(FlashcartState::DISCONNECTED)
                                }
                            }
                        },
                        Err(_) => {
                            dbg_println!("No message from N64 in 10 seconds");
                            Some(FlashcartState::DISCONNECTED)
                        }
                    }
                },
                Some(msg) = struc.rx.recv() => {
                    dbg_println!("Received message from MH, {:?}", msg);
                    match msg {
                        ServerMessage::ItemQueue(items) => {
                            struc.item_queue = items;
                            let item_queue = struc.item_queue.clone();
                            if let InGameState::InGame { ref mut internal_count, ref mut item_pending } = struc.ingame_state {
                                if !*item_pending {
                                    if let Ok(true) = get_item(&item_queue, internal_count).await {
                                        *item_pending = true;
                                    }
                                }
                            }
                        },
                        ServerMessage::GetItem(item) => {
                            struc.item_queue.push(item);
                            let item_queue = struc.item_queue.clone();
                            if let InGameState::InGame { ref mut internal_count, ref mut item_pending } = struc.ingame_state {
                                if !*item_pending {
                                    if let Ok(true) = get_item(&item_queue, internal_count).await {
                                        *item_pending = true;
                                    }
                                }
                            }
                        },
                        ServerMessage::PlayerName(world, new_name) => {
                            let (name, progressive_items) = struc.player_data.entry(world).or_default();
                            *name = new_name;
                            let _ = send_player_data(world, *name, *progressive_items);
                        },
                        ServerMessage::ProgressiveItems(world, new_progressive_items) => {
                            let (name, progressive_items) = struc.player_data.entry(world).or_default();
                            *progressive_items = new_progressive_items;
                            let _ = send_player_data(world, *name, *progressive_items);
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
    }

    fn stream(self: Box<Self>, _: EventStream) -> Pin<Box<dyn Stream<Item = Message> + Send>> {
        let log = self.log || DEBUG_LOGGING;
        stream::try_unfold(FlashcartState::SEARCHING, move |state| async move {
            let _ = sleep(Duration::from_millis(1)).await;
            let mut messages: Vec<Message> = Vec::new();

            let new_state = match &state {
                FlashcartState::DISCONNECTED => {
                    if log {
                        let _ = lock!(log = crate::LOG; writeln!(&*log, "{} EverDrive: waiting 5 seconds before next scan", Utc::now().format("%Y-%m-%d %H:%M:%S")));
                    }
                    if n64flashcart::isopen() {
                        n64flashcart::close();
                    }
                    let _ = sleep(Duration::from_secs(5)).await;
                    Some(FlashcartState::SEARCHING)
                },
                FlashcartState::SEARCHING => {
                    let status = n64flashcart::find();
                    if status == DeviceError::CARTFINDFAIL {
                        n64flashcart::initialize();
                        n64flashcart::set_protocol(ProtocolVer::VERSION2);
                        Some(FlashcartState::DISCONNECTED)
                    } else if status != DeviceError::OK {
                        Some(FlashcartState::DISCONNECTED)
                    } else {
                        let cart_name = n64flashcart::cart_type_to_str(n64flashcart::get_cart());
                        Some(FlashcartState::OPENING(cart_name.to_string()))
                    }
                },
                FlashcartState::OPENING(name) => {
                    let status = n64flashcart::open();
                    if status != DeviceError::OK {
                        dbg_println!("Failed to open USB connection to flashcart, retrying, error code {}", status.value());
                        Some(FlashcartState::DISCONNECTED)
                    } else {
                        dbg_println!("Flashcart USB connection opened");
                        Some(FlashcartState::CONNECTED(name.to_owned(), CommState::WaitForGame))
                    }
                },
                FlashcartState::CONNECTED(name, comm_state) => {
                    match read(name, comm_state).await {
                        Ok((next_state, m)) => {
                            messages.extend(m);
                            next_state
                        },
                        Err(e) => {
                            messages.push(Message::FlashcartCommError(Arc::new(vec![(name.to_owned(), ConnectError::DeviceError(e))])));
                            Some(FlashcartState::CONNECTED(name.to_owned(), comm_state.to_owned()))
                        }
                    }
                }
            };

            if let Some(value) = &new_state {
                messages.push(Message::FlashcartStateChanged(value.clone()));
            }

            Ok::<_, ConnectError>(Some((stream::iter(messages).map(Ok::<_, ConnectError>), new_state.unwrap_or(state))))
        }).try_flatten().map(|res| {
            let mut print_debug = true;

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
