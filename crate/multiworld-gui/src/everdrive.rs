use {
    std::{
        any::TypeId,
        hash::Hash as _,
        pin::Pin,
    },
    futures::stream::{
        self,
        Stream,
        StreamExt as _,
    },
    iced::advanced::subscription::{
        EventStream,
        Recipe,
    },
    crate::Message,
};

pub(crate) struct Subscription;

impl Recipe for Subscription {
    type Output = Message;

    fn hash(&self, state: &mut iced::advanced::Hasher) {
        TypeId::of::<Self>().hash(state);
    }

    fn stream(self: Box<Self>, _: EventStream) -> Pin<Box<dyn Stream<Item = Message> + Send>> {
        stream::pending().boxed() //TODO
    }
}
