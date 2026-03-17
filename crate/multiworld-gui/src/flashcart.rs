use {
    crate::Message, futures::stream::{
            self,
            Stream,
            StreamExt as _,
    }, iced::advanced::subscription::{
        EventStream,
        Recipe,
    }, n64flashcart, std::{
        any::TypeId, hash::Hash as _, pin::Pin, sync::Arc
    }, tokio::time::sleep,
    std::time::Duration,
};

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
    WaitForGame,
    Handshake,
    Idle
}

#[derive(Debug, Clone)]
pub enum FlashcartState {
    DISCONNECTED,
    SEARCHING,
    OPENING(String),
    CONNECTED(String, CommState)
}


const FIVE_SECONDS : Duration = Duration::new(5, 0);

fn send_hanshake() {
    let msg = "cmdt".as_bytes().to_vec();
    let header = n64flashcart::Header { datatype: n64flashcart::USBDataType::TEXT, length: msg.len() };

    let _ = n64flashcart::write(header, msg);                        
}

fn process_message(comm_state: &CommState, header: n64flashcart::Header, data: Vec<u8>) -> CommState {
    match comm_state {
        CommState::WaitForGame => {
            println!("Wait For Game");
            if header.datatype == n64flashcart::USBDataType::HANDSHAKE {
                send_hanshake();
                CommState::Handshake
            } else {
                CommState::WaitForGame
            }
        }
        CommState::Handshake => {
            if data.len() < 16 {
                println!("Invalid handshake reply, restarting handshake");
                CommState::WaitForGame
            } else if data[0] != b'O' || data[1] != b'o' || data[2] != b'T' || data[3] != b'R' {
                println!("Invalid handshake reply, restarting handshake");
                CommState::WaitForGame
            } else {
                let protocol_version = data[4];
                let mut msg = "MW".as_bytes().to_vec();
                msg.push(protocol_version);
                msg.push(0); // MW_SEND_OWN_ITEMS
                msg.push(0); // MW_PROGRESSIVE_ITEMS_ENABLE
                let header = n64flashcart::Header { datatype: n64flashcart::USBDataType::RAWBINARY, length: msg.len() };
                println!("Handshake reply received. Repeating protocol version to finalize handshake");
                let status = n64flashcart::write(header, msg);
                if status == n64flashcart::DeviceError::OK {
                    println!("Protocol version sent");
                    CommState::Idle
                } else {
                    println!("Failed to send protocol version, restarting handshake");
                    CommState::WaitForGame
                }
            }
        },
        CommState::Idle => { CommState::Idle }
    }
}

async fn read(name: &String, comm_state: &CommState) -> Option<FlashcartState> {
    //self.comm_state = CommState::Unknown;
    match n64flashcart::read() {
        Ok((header, data)) => {
            if header.datatype == n64flashcart::USBDataType::EMPTY {
                // Do nothing, no data
            }
            else {
                println!("Datatype: {:?}, Length: {}, data: {:?}", header.datatype, header.length, data);
                let new_comm_state = process_message(comm_state, header, data);
                return Some(FlashcartState::CONNECTED(name.to_owned(), new_comm_state));
            }
        }
        Err(e) => {
            println!("Read error while waiting for handshake, {}", e.value());
            //sleep(FIVE_SECONDS).await;
            return Some(FlashcartState::DISCONNECTED);
        }
    }

    return None;
}

impl Recipe for Subscription {
    type Output = Message;

    fn hash(&self, state: &mut iced::advanced::subscription::Hasher) {
        TypeId::of::<Self>().hash(state);
    }

    fn stream(self: Box<Self>, _: EventStream) -> Pin<Box<dyn Stream<Item = Message> + Send>> {
        stream::try_unfold(FlashcartState::SEARCHING, |state| async move {
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
                        Some(FlashcartState::CONNECTED(name.to_owned(), CommState::WaitForGame))
                    }    
                },
                FlashcartState::CONNECTED(name, comm_state) => {
                    read(name, comm_state).await
                }
            };

            match new_state {
                Some(value) => Ok(Some((Message::FlashcartStateChanged(value.clone()), value.clone()))),
                None => Ok(Some((Message::Nop, state.clone())))
            }
        }).map(|res| res.unwrap_or_else(|e: Error | Message::FrontendSubscriptionError(Arc::new(e.into())))).boxed()
    }
}
