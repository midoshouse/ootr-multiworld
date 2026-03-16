use {
    crate::Message, futures::stream::{
            self,
            Stream,
            StreamExt as _,
    }, iced::advanced::subscription::{
        EventStream,
        Recipe,
    }, n64flashcart, std::{
        any::TypeId,
        hash::Hash as _,
        pin::Pin, sync::Arc,
    }
};

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("N64 reported as world 0")]
    NotFound,
}

pub(crate) struct Subscription {
//    pub(crate) log: bool,
}

impl Recipe for Subscription {
    type Output = Message;

    fn hash(&self, state: &mut iced::advanced::subscription::Hasher) {
        TypeId::of::<Self>().hash(state);
    }

    fn stream(self: Box<Self>, _: EventStream) -> Pin<Box<dyn Stream<Item = Message> + Send>> {
        println!("Flashcart stream");

        stream::try_unfold(0, |_state| async move {
            let status = n64flashcart::find();
            if status == n64flashcart::DeviceError::CARTFINDFAIL {
                println!("Flashcart disconnected, resetting");
                n64flashcart::initialize();
                return Err(Error::NotFound)
            } else if status != n64flashcart::DeviceError::OK {
                // Flashcart not found, wait and retry
                // sleep(Duration(1));
                println!("Flashcart not found");
            } else {
                println!("Flashcart found, {}", n64flashcart::cart_type_to_str(n64flashcart::get_cart()));
            }
            return Ok(None)
        }).map(|res| res.unwrap_or_else(|e: Error | Message::FrontendSubscriptionError(Arc::new(e.into())))).boxed()
    }
}
