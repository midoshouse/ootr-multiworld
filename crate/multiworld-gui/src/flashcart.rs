use {
    crate::{FrontendWriter, Message}, arrayref::array_ref, enum_iterator::all, futures::{TryStreamExt as _, stream::{
            self,
            Stream,
            StreamExt as _,
    }}, iced::advanced::subscription::{
        EventStream,
        Recipe,
    }, multiworld::{Filename, frontend::ClientMessage}, n64flashcart, ootr_utils::spoiler::HashIcon, std::{
        any::TypeId, hash::Hash as _, num::NonZeroU8, pin::Pin, sync::Arc, time::Duration
    }, tokio::{select, sync::{Mutex, mpsc}, time::{sleep, timeout}}
};

const PROTOCOL_VERSION: u8 = 1;
const MW_SEND_OWN_ITEMS: u8 = 1;
const MW_PROGRESSIVE_ITEMS_ENABLE: u8 = 1;

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("Flashcart disconnected")]
    #[expect(dead_code)]
    Disconnected,
}

pub(crate) struct Subscription {
//    pub(crate) log: bool,
}
 
#[derive(Debug, Clone)]
pub(crate) enum CommState {
    SendHandshake,
    WaitForGame,
    Handshake,
    Ready(Arc<Mutex<mpsc::Receiver<multiworld::frontend::ServerMessage>>>)
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
                            println!("Handshake reply received. Repeating protocol version to finalize handshake");
                            let res = send_handshake_response();
                            match res {
                                Ok(_) => {
                                    println!("Protocol version sent");

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

                                    Some(CommState::Ready(Arc::new(Mutex::new(rx))))
                                },
                                Err(_) => {
                                    println!("Failed to send protocol version, restarting handshake");
                                    Some(CommState::WaitForGame)
                                }
                            }
                        },
                        _ => {
                            println!("Invalid handshake reply, restarting handshake");
                            Some(CommState::WaitForGame)
                        }
                    } 
                },
                Err(_) => {
                    println!("Invalid handshake reply, restarting handshake");
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
        CommState::Ready(_rx) => {
            let mut rx = _rx.lock().await;
            let mut messages: Vec<Message> = Vec::new();

            select! {
                res = timeout(Duration::from_secs(5), n64_recv()) => {
                    match res {
                        Ok(res) => {
                            match res {
                                Ok((header, data)) => {
                                    println!("Datatype: {:?}, Length: {}", header.datatype, header.length);
                                    if header.datatype == n64flashcart::USBDataType::SAVE_FILENAME {
                                        if data.len() >= 9 {
                                            messages.push(Message::Plugin(Box::new(ClientMessage::PlayerName(Filename(*array_ref![data, 1, 8])))));
                                        }
                                    }
                                    
                                    if header.datatype == n64flashcart::USBDataType::INGAME_STATE {
                                        if let Ok(savedata) = TryInto::<[u8 ; 5200]>::try_into(data) {
                                            messages.push(Message::Plugin(Box::new(ClientMessage::SaveData(savedata))));
                                        }
                                    }
                                },
                                Err(e) => {
                                    println!("Error receiving from n64, {:?}", e);
                                    return (Some(FlashcartState::DISCONNECTED), vec![]);
                                }
                            };
                        },
                        Err(_) => {
                            println!("No message from N64 in 5 seconds");
                            return (Some(FlashcartState::DISCONNECTED), vec![]);
                        }
                    };
                },
                Some(msg) = rx.recv() => {
                    println!("Received message from MH, {:?}", msg);
                    
                }
            };
            
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
                        if header.datatype == n64flashcart::USBDataType::INGAME_STATE {
                            //println!("Datatype: {:?}, Length: {}", header.datatype, header.length); 
                        }
                        else
                        {
                            println!("Datatype: {:?}, Length: {}, data: {:?}", header.datatype, header.length, data);
                        }
                        let (new_comm_state, messages) = process_message(comm_state, header, data);
                        let next_state = new_comm_state.map(|state| FlashcartState::CONNECTED(name.to_owned(), state));
                        return (next_state, messages);
                    }
                }
                Err(e) => {
                    println!("Read error while waiting for handshake, {}", e.value());
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
                        println!("Failed to open USB connection to flashcart, retrying, error code {}", status.value());
                        Some(FlashcartState::DISCONNECTED)
                    } else {
                        println!("Flashcart USB connection opened");
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
        }).try_flatten().map(|res| res.unwrap_or_else(|e| Message::FrontendSubscriptionError(Arc::new(e.into())))).boxed()
    }
}
