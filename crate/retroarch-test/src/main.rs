use {
    std::{
        io::prelude::*,
        net::Ipv4Addr,
        ops::{
            Add,
            Range,
        },
    },
    tokio::net::UdpSocket,
};

const RDRAM_START: u32 = 0x8000_0000;

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

fn start_len<T: Add<Output = T> + Copy>(start: T, len: T) -> Range<T> {
    start..start + len
}

/// The RetroArch UDP API does not seem to be documented,
/// but there is a Python implementation at
/// <https://github.com/eadmaster/console_hiscore/blob/master/tools/retroarchpythonapi.py>
async fn retroarch_read_ram(sock: &UdpSocket, Range { start, end }: Range<u32>) -> Result<Vec<u8>, Error> {
    let len = end - start;
    // make sure we're word-aligned on both ends
    let offset_in_word = start & 0x3;
    let mut aligned_start = (start - offset_in_word) as usize;
    let mut aligned_len = (offset_in_word + len).next_multiple_of(4);
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
    Ok(ram_buf[offset_in_word as usize..(offset_in_word + len) as usize].to_owned())
}

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)] Io(#[from] std::io::Error),
    #[error(transparent)] ParseInt(#[from] std::num::ParseIntError),
}

#[wheel::main]
async fn main() -> Result<(), Error> {
    let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await?;
    sock.connect((Ipv4Addr::LOCALHOST, 55355)).await?;
    println!("{:x?}", retroarch_read_ram(&sock, start_len(0x11a5d0 + 0x1c, 6)).await?);
    Ok(())
}
