#![deny(rust_2018_idioms, unused, unused_crate_dependencies, unused_import_braces, unused_lifetimes, unused_qualifications, warnings)]
#![forbid(unsafe_code)]

use {
    std::{
        convert::Infallible as Never,
        net::IpAddr,
        time::Duration,
    },
    async_proto::Protocol as _,
    crossterm::{
        event::{
            Event,
            EventStream,
            KeyCode,
            KeyEvent,
            KeyModifiers,
        },
        terminal::{
            disable_raw_mode,
            enable_raw_mode,
        },
    },
    futures::stream::StreamExt as _,
    itertools::Itertools as _,
    tokio::{
        net::TcpStream,
        select,
        time::{
            Instant,
            interval_at,
            timeout,
        },
    },
    multiworld::{
        ClientMessage,
        ServerMessage,
        SessionState,
    },
    crate::parse::FromExpr as _,
};

mod parse;

#[derive(Debug, thiserror::Error)]
enum ParseApiKeyError {
    #[error(transparent)] Int(#[from] std::num::ParseIntError),
    #[error("API key had an odd number of characters")]
    ExtraNybble,
    #[error("API key had wrong length")]
    VecLen(Vec<u8>),
}

impl From<Vec<u8>> for ParseApiKeyError {
    fn from(v: Vec<u8>) -> Self {
        Self::VecLen(v)
    }
}

fn parse_api_key(s: &str) -> Result<[u8; 32], ParseApiKeyError> {
    let mut tuples = s.chars().tuples();
    let key = (&mut tuples).map(|(hi, lo)| u8::from_str_radix(&format!("{hi}{lo}"), 16)).try_collect::<_, Vec<_>, _>()?.try_into()?;
    if tuples.into_buffer().next().is_some() { return Err(ParseApiKeyError::ExtraNybble) }
    Ok(key)
}

#[derive(clap::Parser)]
#[clap(version)]
struct Args {
    #[clap(long)]
    server_ip: Option<IpAddr>,
    #[clap(short, long, default_value_t = multiworld::PORT)]
    port: u16,
    id: Option<u64>,
    #[clap(parse(try_from_str = parse_api_key))]
    api_key: Option<[u8; 32]>,
}

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)] Client(#[from] multiworld::ClientError),
    #[error(transparent)] Elapsed(#[from] tokio::time::error::Elapsed),
    #[error(transparent)] Io(#[from] tokio::io::Error),
    #[error(transparent)] Read(#[from] async_proto::ReadError),
    #[error(transparent)] Syn(#[from] syn::Error),
    #[error(transparent)] TryFromSlice(#[from] std::array::TryFromSliceError),
    #[error(transparent)] Write(#[from] async_proto::WriteError),
    #[error("expected exactly one element, got zero or multiple")]
    ExactlyOne,
    #[error("failed to parse")]
    FromExpr,
}

impl<T: Iterator> From<itertools::ExactlyOneError<T>> for Error {
    fn from(_: itertools::ExactlyOneError<T>) -> Self {
        Error::ExactlyOne
    }
}

fn try_disable_raw_mode(e: impl Into<Error>) -> Error {
    match disable_raw_mode() {
        Ok(()) => e.into(),
        Err(e) => e.into(),
    }
}

#[wheel::main(debug)]
async fn main(Args { server_ip, port, id, api_key }: Args) -> Result<(), Error> {
    enable_raw_mode()?;
    let mut cli_events = EventStream::default().fuse();
    let mut tcp_stream = TcpStream::connect((server_ip.unwrap_or(IpAddr::V4(multiworld::ADDRESS_V4)), port)).await.map_err(try_disable_raw_mode)?;
    multiworld::handshake(&mut tcp_stream).await.map_err(try_disable_raw_mode)?;
    if let (Some(id), Some(api_key)) = (id, api_key) {
        ClientMessage::Login { id, api_key }.write(&mut tcp_stream).await.map_err(try_disable_raw_mode)?;
    }
    let (reader, mut writer) = tcp_stream.into_split();
    let mut session_state = SessionState::<Never>::Init;
    let mut read = Box::pin(timeout(Duration::from_secs(60), ServerMessage::read_owned(reader)));
    let mut cmd_buf = String::default();
    let mut interval = interval_at(Instant::now() + Duration::from_secs(30), Duration::from_secs(30));
    loop {
        select! {
            res = &mut read => {
                let (reader, msg) = res.map_err(try_disable_raw_mode)?.map_err(try_disable_raw_mode)?;
                //TODO move to beginning of line, restore position after printing
                println!("{msg:#?}");
                session_state.apply(msg);
                read = Box::pin(timeout(Duration::from_secs(60), ServerMessage::read_owned(reader)));
            },
            cli_event = cli_events.select_next_some() => match cli_event.map_err(try_disable_raw_mode)? {
                Event::Key(key_event) => if key_event == KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL) {
                    break
                } else {
                    match key_event.code {
                        KeyCode::Enter => {
                            ClientMessage::from_expr(syn::parse_str(&cmd_buf).map_err(try_disable_raw_mode)?).map_err(try_disable_raw_mode)?.write(&mut writer).await.map_err(try_disable_raw_mode)?;
                            cmd_buf.clear();
                        }
                        KeyCode::Char(c) => cmd_buf.push(c),
                        _ => {}
                    }
                },
                Event::Paste(text) => cmd_buf.push_str(&text),
                _ => {}
            },
            _ = interval.tick() => ClientMessage::Ping.write(&mut writer).await.map_err(try_disable_raw_mode)?,
        }
    }
    disable_raw_mode()?;
    Ok(())
}
