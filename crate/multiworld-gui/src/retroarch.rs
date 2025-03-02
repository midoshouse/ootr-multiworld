//! The RetroArch UDP API is documented at <https://docs.libretro.com/development/retroarch/network-control-interface/>

use {
    std::{
        any::TypeId,
        //collections::VecDeque,
        hash::Hash as _,
        io::prelude::*,
        net::Ipv4Addr,
        //ops::Range,
        pin::Pin,
        sync::Arc,
    },
    futures::stream::{
        self,
        Stream,
        StreamExt as _,
        //TryStreamExt as _,
    },
    iced::advanced::subscription::{
        EventStream,
        Recipe,
    },
    tokio::net::UdpSocket,
    crate::Message,
};

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error(transparent)] Io(#[from] std::io::Error),
    #[error(transparent)] ParseInt(#[from] std::num::ParseIntError),
    #[error("randomizer version too new (version {coop_context_version}; please tell Fenhl that Mido's House Multiworld needs to be updated)")]
    RandoTooNew {
        coop_context_version: u32,
    },
    #[error("randomizer version too old (version 5.1.4 or higher required)")]
    RandoTooOld {
        coop_context_version: u32,
    },
}

trait IteratorExt {
    fn try_array_chunks<const N: usize>(self) -> impl Iterator;
}

#[allow(refining_impl_trait_internal)]
impl<T, E, I: Iterator<Item = Result<T, E>>> IteratorExt for I {
    fn try_array_chunks<const N: usize>(self) -> impl Iterator<Item = Result<[T; N], E>> {
        struct TryArrayChunks<T, E, I: Iterator<Item = Result<T, E>>, const N: usize> {
            inner: I,
        }

        impl<T, E, I: Iterator<Item = Result<T, E>>, const N: usize> Iterator for TryArrayChunks<T, E, I, N> {
            type Item = Result<[T; N], E>;

            fn next(&mut self) -> Option<Self::Item> {
                let mut buf = [const { None }; N];
                for idx in 0..N {
                    match self.inner.next() {
                        None => return None,
                        Some(Ok(value)) => buf[idx] = Some(value),
                        Some(Err(e)) => return Some(Err(e)),
                    }
                }
                Some(Ok(buf.map(Option::unwrap)))
            }
        }

        assert!(N != 0, "chunk size must be non-zero");
        TryArrayChunks { inner: self }
    }
}

/*
async fn read_dynamic_size_ram(sock: &UdpSocket, Range { start, end }: Range<u32>) -> Result<VecDeque<u8>, Error> {
    let len = end - start;
    // make sure we're word-aligned on both ends
    let offset_in_word = start & 0x3;
    let mut aligned_start = (start - offset_in_word) as usize;
    let mut aligned_len = (offset_in_word + len).next_multiple_of(4);
    let mut packet_buf = [0; 4096];
    let mut ram_buf = VecDeque::with_capacity(aligned_len as usize);
    let mut prefix = Vec::with_capacity(21);
    let mut msg = Vec::with_capacity(26);
    while aligned_len > 0 {
        // make sure the hex-encoded response fits into the 4096-byte buffer RetroArch uses
        // each encoded byte requires 3 bytes of buffer space (the whitespace plus the 2-character hex encoding)
        // and we want to stay word-aligned
        const MAX_ENCODED_BYTES_PER_BUFFER: u32 = ((4_096 - "READ_CORE_RAM ffffffff 9999\n".len() as u32) / 3) & !0x3;

        // using READ_CORE_MEMORY instead of READ_CORE_RAM as suggested in https://github.com/libretro/RetroArch/blob/0357b6c/command.h#L430-L437 fails with “-1 no memory map defined”
        // this may be fixed by https://github.com/libretro/mupen64plus-libretro-nx/pull/545 in the future
        let count = aligned_len.min(MAX_ENCODED_BYTES_PER_BUFFER);
        prefix.clear();
        write!(&mut prefix, "READ_CORE_RAM {aligned_start:x} ")?;
        msg.clear();
        write!(&mut msg, "READ_CORE_RAM {aligned_start:x} ")?;
        writeln!(&mut msg, "{count}")?;
        sock.send(&msg).await?;
        let packet_len = sock.recv(&mut packet_buf).await?;
        let response = &packet_buf[prefix.len()..packet_len - 1];
        let words = response.split(|&sep| sep == b' ').map(|byte| u8::from_str_radix(&String::from_utf8_lossy(byte), 16)).try_array_chunks();
        for res in words {
            let [b3, b2, b1, b0] = res?;
            ram_buf.extend([b0, b1, b2, b3]);
        }
        aligned_start += count as usize;
        aligned_len -= count;
    }
    for _ in 0..offset_in_word {
        ram_buf.pop_front();
    }
    ram_buf.truncate(len as usize);
    Ok(ram_buf)
}
*/

async fn read_fixed_size_ram<const N: usize>(sock: &UdpSocket, start: u32) -> Result<[u8; N], Error> {
    // make sure we're word-aligned on both ends
    let offset_in_word = start & 0x3;
    let mut aligned_start = (start - offset_in_word) as usize;
    let mut aligned_len = (offset_in_word + N as u32).next_multiple_of(4);
    let mut packet_buf = [0; 4096];
    let mut ram_buf = Vec::with_capacity(aligned_len as usize);
    let mut prefix = Vec::with_capacity(21);
    let mut msg = Vec::with_capacity(26);
    while aligned_len > 0 {
        // make sure the hex-encoded response fits into the 4096-byte buffer RetroArch uses
        // each encoded byte requires 3 bytes of buffer space (the whitespace plus the 2-character hex encoding)
        // and we want to stay word-aligned
        const MAX_ENCODED_BYTES_PER_BUFFER: u32 = ((4_096 - "READ_CORE_RAM ffffffff 9999\n".len() as u32) / 3) & !0x3;

        // using READ_CORE_MEMORY instead of READ_CORE_RAM as suggested in https://github.com/libretro/RetroArch/blob/0357b6c/command.h#L430-L437 fails with “-1 no memory map defined”
        // this may be fixed by https://github.com/libretro/mupen64plus-libretro-nx/pull/545 in the future
        let count = aligned_len.min(MAX_ENCODED_BYTES_PER_BUFFER);
        prefix.clear();
        write!(&mut prefix, "READ_CORE_RAM {aligned_start:x} ")?;
        msg.clear();
        write!(&mut msg, "READ_CORE_RAM {aligned_start:x} ")?;
        writeln!(&mut msg, "{count}")?;
        sock.send(&msg).await?;
        let packet_len = sock.recv(&mut packet_buf).await?;
        let response = &packet_buf[prefix.len()..packet_len - 1];
        let words = response.split(|&sep| sep == b' ').map(|byte| u8::from_str_radix(&String::from_utf8_lossy(byte), 16)).try_array_chunks();
        for res in words {
            let [b3, b2, b1, b0] = res?;
            ram_buf.extend_from_slice(&[b0, b1, b2, b3]);
        }
        aligned_start += count as usize;
        aligned_len -= count;
    }
    // manual implementation of array_ref! macro to avoid compiler getting confused about const generic scope
    Ok(ram_buf[offset_in_word as usize..offset_in_word as usize + N].try_into().expect("array ref len and offset should be valid for provided array"))
}

async fn read_u32(sock: &UdpSocket, start: u32) -> Result<u32, Error> {
    Ok(u32::from_be_bytes(read_fixed_size_ram(sock, start).await?))
}

async fn write_fixed_size_ram<const N: usize>(sock: &UdpSocket, start: u32, data: [u8; N]) -> Result<(), Error> {
    if N > 0 {
        let mut packet_buf = [0; 4096];
        let mut prefix = Vec::with_capacity(22);
        let mut msg = Vec::with_capacity(27);

        // make sure the hex-encoded response fits into the 4096-byte buffer RetroArch uses
        // each encoded byte requires 3 bytes of buffer space (the whitespace plus the 2-character hex encoding)
        const MAX_ENCODED_BYTES_PER_BUFFER: u32 = (4_096 - "WRITE_CORE_RAM ffffffff\n".len() as u32) / 3;

        //TODO
    }
}

pub(crate) struct Subscription;

impl Recipe for Subscription {
    type Output = Message;

    fn hash(&self, state: &mut iced::advanced::subscription::Hasher) {
        TypeId::of::<Self>().hash(state);
    }

    fn stream(self: Box<Self>, _: EventStream) -> Pin<Box<dyn Stream<Item = Message> + Send>> {
        enum SubscriptionState {
            Init,
        }

        stream::try_unfold(SubscriptionState::Init, |state| async move {
            match state {
                SubscriptionState::Init => {
                    let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await?;
                    sock.connect((Ipv4Addr::LOCALHOST, 55355)).await?;
                    let zeldaz_rdram = read_fixed_size_ram::<6>(&sock, 0x11a5d0 + 0x1c).await?;
                    //let mut coop_context_addr = None;
                    if zeldaz_rdram == *b"ZELDAZ" {
                        let rando_context_addr = read_u32(&sock, 0x1c6e90 + 0x15d4).await?;
                        if rando_context_addr >= 0x8000_0000 && rando_context_addr != 0xffff_ffff {
                            let new_coop_context_addr = read_u32(&sock, rando_context_addr - 0x8000_0000);
                            if new_coop_context_addr >= 0x8000_0000 && new_coop_context_addr != 0xffff_ffff {
                                let coop_context_version = read_u32(&sock, new_coop_context_addr - 0x8000_0000);
                                if coop_context_version < 2 {
                                    return Err(Error::RandoTooOld { coop_context_version })
                                }
                                if coop_context_version > 7 {
                                    return Err(Error::RandoTooNew { coop_context_version })
                                }
                                if coop_context_version >= 3 {
                                    write_u8(&sock, (new_coop_context_addr - 0x8000_0000) + 0x000a, 1).await?; // enable MW_SEND_OWN_ITEMS for server-side tracking
                                }
                            }
                        }
                    }
                }
            }
            Ok::<_, Error>(None::<(Message, SubscriptionState)>) //TODO
        }).map(|res| res.unwrap_or_else(|e| Message::FrontendSubscriptionError(Arc::new(e.into())))).boxed()

    }
}
