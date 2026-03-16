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
pub enum FlashcartState {
    DISCONNECTED,
    SEARCHING,
    OPENING(String),
    CONNECTED(String)
}

const FIVE_SECONDS : Duration = Duration::new(5, 0);

async fn read() -> Option<FlashcartState> {
    //self.game_state = GameState::Unknown;
    match n64flashcart::read() {
        Ok((header, data)) => {
            if header.datatype == n64flashcart::USBDataType::HANDSHAKE || header.datatype == n64flashcart::USBDataType::HEARTBEAT {
                println!("Handshake request detected");
            } else if header.datatype == n64flashcart::USBDataType::EMPTY {
                println!("No data to read while waiting for handshake");
                sleep(FIVE_SECONDS).await;
            } else {
                println!("Invalid handshake, {}, {}", header.datatype.value(), data.iter().map(|b| format!("{:02x}", b)).collect::<Vec<String>>().join(" "));
                //sleep(FIVE_SECONDS).await;
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
                        Some(FlashcartState::CONNECTED(name.clone()))
                    }    
                },
                FlashcartState::CONNECTED(_name) => {
                    read().await
                }
            };

            match new_state {
                Some(value) => Ok(Some((Message::FlashcartStateChanged(value.clone()), value.clone()))),
                None => Ok(Some((Message::Nop, state.clone())))
            }
        }).map(|res| res.unwrap_or_else(|e: Error | Message::FrontendSubscriptionError(Arc::new(e.into())))).boxed()
    }
}
