use {
    crate::{FrontendWriter, Message}, arrayref::{array_mut_ref, array_ref}, enum_iterator::all, futures::{TryStreamExt as _, stream::{
            self,
            Stream,
            StreamExt as _,
    }}, iced::advanced::subscription::{
        EventStream,
        Recipe,
    }, multiworld::{Filename, frontend::{ClientMessage, ServerMessage}}, n64flashcart, ootr_utils::spoiler::HashIcon, std::{
        any::TypeId, collections::{HashMap, VecDeque}, hash::Hash as _, num::NonZeroU8, pin::Pin, sync::Arc, time::Duration
    }, tokio::{select, sync::{Mutex, mpsc}, time::{sleep, timeout}}
};

const DEBUG_LOGGING: bool = true;

macro_rules! dbg_println {
    ($($arg:tt)*) => {
        if DEBUG_LOGGING {
            println!($($arg)*);
        }
    };
}

const PROTOCOL_VERSION: u8 = 1;
const MW_SEND_OWN_ITEMS: u8 = 1;
const MW_PROGRESSIVE_ITEMS_ENABLE: u8 = 1;

const CMD_PLAYER_DATA: u8 = 1;
const CMD_GET_ITEM: u8 = 2;
const N64_ITEM_SEND: u8 = 3;
const N64_ITEM_RECEIVED: u8 = 4;

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("Flashcart disconnected")]
    #[expect(dead_code)]
    Disconnected,
    #[error("received item for world 0")]
    PlayerId,
}

pub(crate) struct Subscription {
//    pub(crate) log: bool,
}

#[derive(Debug, Clone)]
pub(crate) enum InGameState {
    Unknown,
    FileSelect,
    InGame,
    Receiving
}

#[derive(Debug)]
pub(crate) struct InGameStruct {
    rx: mpsc::Receiver<ServerMessage>,
    item_queue: VecDeque<u16>,
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

const FIVE_SECONDS : Duration = Duration::new(5, 0);

fn send_handshake() {
    let msg = "cmdt".as_bytes().to_vec();
    let header = n64flashcart::Header { datatype: n64flashcart::USBDataType::TEXT, length: msg.len() };

    let _ = n64flashcart::write(header, msg);
}

fn send_handshake_response() -> Result<(), n64flashcart::DeviceError>  {
    let mut msg = "MW".as_bytes().to_vec();
    msg.push(PROTOCOL_VERSION);
    msg.push(MW_SEND_OWN_ITEMS); // MW_SEND_OWN_ITEMS
    msg.push(MW_PROGRESSIVE_ITEMS_ENABLE); // MW_PROGRESSIVE_ITEMS_ENABLE
    let header = n64flashcart::Header { datatype: n64flashcart::USBDataType::RAWBINARY, length: msg.len() };
    let status = n64flashcart::write(header, msg);
    
    match status {
        n64flashcart::DeviceError::OK => Ok(()),
        _ => Err(status)
    }
}

async fn n64_recv() -> Result<(n64flashcart::Header, Vec<u8>), n64flashcart::DeviceError> {
    loop {
        match n64flashcart::read() {
            Ok((header, data)) => {
                if header.datatype != n64flashcart::USBDataType::EMPTY {
                    return Ok((header, data));
                }
            }
            Err(e) => return Err(e),
        };
    }
}

fn n64_send(datatype: n64flashcart::USBDataType, msg: Vec<u8>) -> Result<(), n64flashcart::DeviceError> {
    let header = n64flashcart::Header { datatype: datatype, length: msg.len() };
    let status = n64flashcart::write(header, msg);
    match status {
        n64flashcart::DeviceError::OK => Ok(()),
        _ => Err(status)
    }
}

fn send_player_data(world: NonZeroU8, name: Filename, progressive_items: u32) -> Result<(), n64flashcart::DeviceError> {
    let mut buf = [0; 16];

    buf[0] = CMD_PLAYER_DATA;
    buf[1] = world.get();
    *array_mut_ref![buf, 2, 8] = name.0;
    *array_mut_ref![buf, 10, 4] = progressive_items.to_be_bytes();

    n64_send(n64flashcart::USBDataType::RAWBINARY, Vec::from(buf))
}

fn send_item(item: u16) -> Result<(), n64flashcart::DeviceError> {
    let mut msg: Vec<u8> = Vec::new();

    msg.push(CMD_GET_ITEM);

    let [b1, b2] = item.to_be_bytes();
    msg.push(b1);
    msg.push(b2);

    n64_send(n64flashcart::USBDataType::RAWBINARY, msg)
}

fn process_n64_packet(header: n64flashcart::Header, data: Vec<u8>, struc: &mut InGameStruct) -> (Option<InGameState>, Option<Vec<Message>>)
{
    match header.datatype {
        n64flashcart::USBDataType::INGAME_STATE => dbg_println!("Datatype: {:?}, Length: {}", header.datatype, header.length),
        _ => dbg_println!("Datatype: {:?}, Length: {}, data: {:?}", header.datatype, header.length, data)
    };

    match header.datatype {
        n64flashcart::USBDataType::SAVE_FILENAME => {
            if data.len() >= 9 {
                let data_slice = data.into_boxed_slice();
                let fname = *array_ref![data_slice, 1, 8];

                match fname {
                    [0, 0, 0, 0, 0, 0, 0, 0] => {
                        (Some(InGameState::FileSelect), None)
                    },
                    _ => {
                        let filename = Filename(fname);
                        if let None = struc.filename {
                            struc.filename = Some(filename);
                        }

                        (Some(InGameState::InGame), Some(vec![Message::Plugin(Box::new(ClientMessage::PlayerName(filename)))]))
                    }

                }
            }
            else {
                (None, None)
            }
        },

        n64flashcart::USBDataType::INGAME_STATE => {
            if let Ok(savedata) = TryInto::<[u8 ; 5200]>::try_into(data) {
                let mut messages = Vec::new();
                
                if let None = struc.filename {
                    let filename = Filename(*array_ref![savedata, 0x024, 8]);
                    struc.filename = Some(filename);
                    messages.push(Message::Plugin(Box::new(ClientMessage::PlayerName(filename))));
                }

                messages.push(Message::Plugin(Box::new(ClientMessage::SaveData(savedata))));

                (Some(InGameState::InGame), Some(messages))
            } else {
                (None, None)
            }            
        },

        n64flashcart::USBDataType::RAWBINARY =>  {
            match data[0] {
                N64_ITEM_SEND => {  // Item sent
                    let data_slice = data.into_boxed_slice();

                    let kind = u16::from_be_bytes(*array_ref![data_slice, 9, 2]);
                    let target_world = NonZeroU8::new(data_slice[11]).ok_or(Error::PlayerId).unwrap();

                    dbg_println!("Got item {} for world {}", kind, target_world);

                    let message = Message::Plugin(Box::new(ClientMessage::SendItem {
                        key: u64::from_be_bytes(*array_ref![data_slice, 1, 8]),
                        kind: kind,
                        target_world: target_world,
                    }));

                    (None, Some(vec![message]))
                }
                N64_ITEM_RECEIVED => (Some(InGameState::InGame), None), // Item receive confirmation
                _ => (None, None)
            }
        },

        _ => (None, None)
    }
}

fn process_message(comm_state: &CommState, header: n64flashcart::Header, data: Vec<u8>) -> (Option<CommState>, Vec<Message>) {
    let mut messages = Vec::new();
    let next_comm_state = match comm_state {
        CommState::SendHandshake => {
            send_handshake();
            Some(CommState::Handshake)
        }
        CommState::WaitForGame => {
            if header.datatype == n64flashcart::USBDataType::HANDSHAKE {
                Some(CommState::SendHandshake)
            } else {
                None
            }
        }
        CommState::Handshake => {
            match TryInto::<[u8 ; 16]>::try_into(data) {
                Ok(value) => {
                    match value {
                        [b'O', b'o', b'T', b'R', PROTOCOL_VERSION, major, minor, patch, branch, supplementary, player_id, hash1, hash2, hash3, hash4, hash5] => {
                            dbg_println!("Handshake reply received. Repeating protocol version to finalize handshake");
                            let res = send_handshake_response();
                            match res {
                                Ok(_) => {
                                    dbg_println!("Protocol version sent");

                                    let _version = ootr_utils::Version::from_bytes([major, minor, patch, branch, supplementary]).unwrap();
                                    let player_id = NonZeroU8::new(player_id).unwrap();
                                    let file_hash: [HashIcon; 5] = [
                                        all().nth(hash1.into()).unwrap(),
                                        all().nth(hash2.into()).unwrap(),
                                        all().nth(hash3.into()).unwrap(),
                                        all().nth(hash4.into()).unwrap(),
                                        all().nth(hash5.into()).unwrap(),
                                    ];
                                    
                                    let (tx, rx) = mpsc::channel(1_024);

                                    messages.push(Message::FrontendConnected(FrontendWriter::Mpsc(tx)));
                                    messages.push(Message::Plugin(Box::new(ClientMessage::PlayerId(player_id))));
                                    messages.push(Message::Plugin(Box::new(ClientMessage::FileHash(Some(file_hash)))));

                                    let struc = InGameStruct {
                                        rx: rx,
                                        item_queue: VecDeque::new(),
                                        ingame_state: InGameState::Unknown,
                                        filename: None,
                                        player_data: HashMap::default()
                                    };

                                    Some(CommState::Ready(Arc::new(Mutex::new(struc))))
                                },
                                Err(_) => {
                                    dbg_println!("Failed to send protocol version, restarting handshake");
                                    Some(CommState::WaitForGame)
                                }
                            }
                        },
                        _ => {
                            dbg_println!("Invalid handshake reply, restarting handshake");
                            Some(CommState::WaitForGame)
                        }
                    } 
                },
                Err(_) => {
                    dbg_println!("Invalid handshake reply, restarting handshake");
                    Some(CommState::WaitForGame)
                }
            }
        },
        CommState::Ready(_) => None,
    };

    (next_comm_state, messages)
}

async fn read(name: &String, comm_state: &CommState) -> (Option<FlashcartState>, Vec<Message>) {
    let next_state = match comm_state {
        CommState::SendHandshake => {
            send_handshake();
            Some(FlashcartState::CONNECTED(name.to_owned(), CommState::Handshake))
        },
        CommState::Ready(_struc) => {
            let mut struc = _struc.lock().await;
            let mut messages = Vec::new();

            if !struc.item_queue.is_empty() {
                if let InGameState::InGame = struc.ingame_state {
                    let item = struc.item_queue.pop_front().unwrap();
                    let _ = send_item(item);

                    struc.ingame_state = InGameState::Receiving;
                }
            }

            select! {
                n64_or_timeout = timeout(Duration::from_secs(5), n64_recv()) => {
                    match n64_or_timeout {
                        Ok(n64_result) => {
                            match n64_result {
                                Ok((header, data)) => {
                                    let (state_, messages_) = process_n64_packet(header, data, &mut struc);
                                    if let Some(value) = state_ {
                                        struc.ingame_state = value;
                                    }
                                    if let Some(msg) = messages_ {
                                        messages.extend(msg);
                                    }
                                },
                                Err(e) => {
                                    dbg_println!("Error receiving from n64, {:?}", e);
                                    return (Some(FlashcartState::DISCONNECTED), vec![]);
                                }
                            }
                        },
                        Err(_) => {
                            dbg_println!("No message from N64 in 5 seconds");
                            return (Some(FlashcartState::DISCONNECTED), vec![]);
                        }
                    };
                },
                Some(msg) = struc.rx.recv() => {
                    dbg_println!("Received message from MH, {:?}", msg);
                    match msg {
                        ServerMessage::ItemQueue(items) => {
                            struc.item_queue.extend(items);
                        },
                        ServerMessage::GetItem(item) => {
                            struc.item_queue.push_back(item);
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
                    
                }
            };
            
            dbg_println!("InGameState: {:?}", struc.ingame_state);
            return (None, messages);
        },
        _ => {
            match n64flashcart::read() {
                Ok((header, data)) => {
                    if header.datatype == n64flashcart::USBDataType::EMPTY {
                        // Do nothing, no data
                        None
                    }
                    else {
                        let (new_comm_state, messages) = process_message(comm_state, header, data);
                        let next_state = new_comm_state.map(|state| FlashcartState::CONNECTED(name.to_owned(), state));
                        return (next_state, messages);
                    }
                }
                Err(e) => {
                    dbg_println!("Read error while waiting for handshake, {}", e.value());
                    //sleep(FIVE_SECONDS).await;
                    Some(FlashcartState::DISCONNECTED)
                }
            }
        }
    };

    (next_state, vec![])
}

impl Recipe for Subscription {
    type Output = Message;

    fn hash(&self, state: &mut iced::advanced::subscription::Hasher) {
        TypeId::of::<Self>().hash(state);
    }

    fn stream(self: Box<Self>, _: EventStream) -> Pin<Box<dyn Stream<Item = Message> + Send>> {
        stream::try_unfold(FlashcartState::SEARCHING, |state| async move {
            let _ = sleep(Duration::from_millis(100)).await;
            let mut messages: Vec<Message> = Vec::new();

            let new_state = match &state {
                FlashcartState::DISCONNECTED => {
                    let _ = sleep(FIVE_SECONDS).await;
                    Some(FlashcartState::SEARCHING)
                },
                FlashcartState::SEARCHING => {
                    let status = n64flashcart::find();
                    if status == n64flashcart::DeviceError::CARTFINDFAIL {
                        n64flashcart::initialize();
                        Some(FlashcartState::DISCONNECTED)
                    } else if status != n64flashcart::DeviceError::OK {
                        Some(FlashcartState::DISCONNECTED)
                    } else {
                        let cart_name = n64flashcart::cart_type_to_str(n64flashcart::get_cart());
                        Some(FlashcartState::OPENING(cart_name.to_string()))
                    }
                },
                FlashcartState::OPENING(name) => {
                    let status = n64flashcart::open();
                    if status != n64flashcart::DeviceError::OK {
                        dbg_println!("Failed to open USB connection to flashcart, retrying, error code {}", status.value());
                        Some(FlashcartState::DISCONNECTED)
                    } else {
                        dbg_println!("Flashcart USB connection opened");
                        Some(FlashcartState::CONNECTED(name.to_owned(), CommState::SendHandshake))
                    }    
                },
                FlashcartState::CONNECTED(name, comm_state) => {
                    let (next_state, m) = read(name, comm_state).await;
                    messages.extend(m);
                    next_state
                }
            };

            if let Some(value) = &new_state {
                messages.push(Message::FlashcartStateChanged(value.clone()));
            }

            Ok::<_, Error>(Some((stream::iter(messages).map(Ok::<_, Error>), new_state.unwrap_or(state))))
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
