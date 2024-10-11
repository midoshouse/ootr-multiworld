use {
    std::{
        borrow::Cow,
        convert::Infallible as Never,
        io::stdout,
        time::Duration,
    },
    async_proto::Protocol as _,
    chrono::prelude::*,
    crossterm::{
        cursor::{
            MoveLeft,
            MoveToColumn,
        },
        event::{
            Event,
            EventStream,
            KeyCode,
            KeyEvent,
            KeyEventKind,
            KeyModifiers,
        },
        style::Print,
        terminal::{
            Clear,
            ClearType,
            disable_raw_mode,
            enable_raw_mode,
        },
    },
    futures::stream::StreamExt as _,
    tokio::{
        select,
        time::{
            Instant,
            interval_at,
            timeout,
        },
    },
    tokio_tungstenite::tungstenite,
    multiworld::{
        SessionState,
        config::Config,
        user_agent,
        ws::latest::{
            ClientMessage,
            ServerMessage,
        },
    },
    crate::parse::FromExpr as _,
};

mod parse;

#[derive(clap::Parser)]
#[clap(version)]
struct Args {
    api_key: Option<String>,
}

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)] Client(#[from] multiworld::ClientError),
    #[error(transparent)] Config(#[from] multiworld::config::Error),
    #[error(transparent)] Elapsed(#[from] tokio::time::error::Elapsed),
    #[error(transparent)] FilenameParse(#[from] multiworld::FilenameParseError),
    #[error(transparent)] Http(#[from] tungstenite::http::Error),
    #[error(transparent)] InvalidUri(#[from] tungstenite::http::uri::InvalidUri),
    #[error(transparent)] Io(#[from] tokio::io::Error),
    #[error(transparent)] Json(#[from] serde_json::Error),
    #[error(transparent)] Read(#[from] async_proto::ReadError),
    #[error(transparent)] Syn(#[from] syn::Error),
    #[error(transparent)] TryFromSlice(#[from] std::array::TryFromSliceError),
    #[error(transparent)] UrlParse(#[from] url::ParseError),
    #[error(transparent)] WebSocket(#[from] tungstenite::Error),
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

fn prompt(session_state: &SessionState<Never>) -> Cow<'static, str> {
    match session_state {
        SessionState::Error { .. } => Cow::Borrowed("error"),
        SessionState::Init { .. } => Cow::Borrowed("connecting"),
        SessionState::InitAutoRejoin { .. } => Cow::Borrowed("connecting (will auto-rejoin)"),
        SessionState::Lobby { login_state, rooms, wrong_password, .. } => Cow::Owned(format!("lobby ({} room{}{}{})",
            rooms.len(),
            if rooms.len() == 1 { "" } else { "s" },
            if let Some(login_state) = login_state { if login_state.admin { ", admin" } else { ", signed in" } } else { "" },
            if *wrong_password { ", wrong room password" } else { "" },
        )),
        SessionState::Room { room_name, players, num_unassigned_clients, item_queue, .. } => Cow::Owned(format!("room {room_name:?} ({}{}{}{})",
            if players.is_empty() { Cow::Borrowed("") } else if players.len() == 1 { Cow::Borrowed("1 player") } else { Cow::Owned(format!("{} players", players.len())) },
            match (players.is_empty(), *num_unassigned_clients == 0) {
                (false, false) => ", ",
                (true, true) => "empty",
                (_, _) => "",
            },
            match num_unassigned_clients { 0 => Cow::Borrowed(""), 1 => Cow::Borrowed("1 unassigned client"), _ => Cow::Owned(format!("{num_unassigned_clients} unassigned clients")) },
            if item_queue.is_empty() { Cow::Borrowed("") } else if item_queue.len() == 1 { Cow::Borrowed(", 1 item") } else { Cow::Owned(format!(", {} items", item_queue.len())) },
        )),
        SessionState::Closed { .. } => Cow::Borrowed("closed"),
    }
}

async fn cli(Args { api_key }: Args) -> Result<(), Error> {
    let mut cli_events = EventStream::default().fuse();
    let config = Config::load().await?;
    let request = tungstenite::ClientRequestBuilder::new(config.websocket_url()?.to_string().try_into()?)
        .with_header(tungstenite::http::header::USER_AGENT.to_string(), user_agent());
    let (mut websocket, _) = tokio_tungstenite::connect_async(request).await?;
    if let Some(api_key) = api_key {
        ClientMessage::LoginApiKey { api_key }.write_ws024(&mut websocket).await?;
    }
    let (mut writer, reader) = websocket.split();
    let mut session_state = SessionState::<Never>::default();
    let mut read = Box::pin(timeout(Duration::from_secs(60), ServerMessage::read_ws_owned024(reader)));
    let mut cmd_buf = String::default();
    let mut interval = interval_at(Instant::now() + Duration::from_secs(30), Duration::from_secs(30));
    let mut stdout = stdout();
    crossterm::execute!(stdout,
        Print(format_args!("{}> ", prompt(&session_state))),
    )?;
    loop {
        select! {
            res = &mut read => {
                let (reader, msg) = res??;
                session_state.apply(msg.clone());
                if !matches!(msg, ServerMessage::Ping) {
                    crossterm::execute!(stdout,
                        MoveToColumn(0),
                        Clear(ClearType::UntilNewLine),
                        Print(format_args!("{} {msg:#?}\r\n{}> {cmd_buf}", Local::now().format("%Y-%m-%d %H:%M:%S"), prompt(&session_state))),
                    )?;
                }
                read = Box::pin(timeout(Duration::from_secs(60), ServerMessage::read_ws_owned024(reader)));
            },
            cli_event = cli_events.select_next_some() => match cli_event? {
                Event::Key(key_event) => if key_event == KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL) {
                    break
                } else {
                    match key_event.code {
                        KeyCode::Enter => if key_event.kind == KeyEventKind::Press {
                            if !cmd_buf.is_empty() {
                                ClientMessage::from_expr(syn::parse_str(&cmd_buf)?)?.write_ws024(&mut writer).await?;
                            }
                            cmd_buf.clear();
                            crossterm::execute!(stdout,
                                Print(format_args!("\r\n{}> ", prompt(&session_state))),
                            )?;
                        },
                        KeyCode::Backspace => if matches!(key_event.kind, KeyEventKind::Press | KeyEventKind::Repeat) && cmd_buf.pop().is_some() {
                            crossterm::execute!(stdout,
                                MoveLeft(1),
                                Clear(ClearType::UntilNewLine),
                            )?;
                        },
                        KeyCode::Char(c) => if matches!(key_event.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
                            cmd_buf.push(c);
                            crossterm::execute!(stdout,
                                Print(c),
                            )?;
                        },
                        _ => {}
                    }
                },
                Event::Paste(text) => cmd_buf.push_str(&text),
                _ => {}
            },
            _ = interval.tick() => ClientMessage::Ping.write_ws024(&mut writer).await?,
        }
    }
    Ok(())
}

#[wheel::main]
async fn main(args: Args) -> Result<(), Error> {
    enable_raw_mode()?;
    let res = cli(args).await;
    disable_raw_mode()?;
    res
}
