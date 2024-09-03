use {
    std::{
        any::TypeId,
        collections::HashMap,
        hash::Hash as _,
        io::prelude::*,
        mem,
        num::NonZeroU8,
        pin::Pin,
        sync::Arc,
        time::Duration,
    },
    arrayref::{
        array_mut_ref,
        array_ref,
    },
    chrono::prelude::*,
    enum_iterator::all,
    futures::{
        future::{
            Future,
            FutureExt as _,
        },
        stream::{
            self,
            Stream,
            StreamExt as _,
            TryStreamExt as _,
        },
    },
    iced::advanced::subscription::{
        EventStream,
        Recipe,
    },
    log_lock::lock,
    ootr_utils::spoiler::HashIcon,
    tokio::{
        io::{
            self,
            AsyncReadExt,
            AsyncWriteExt,
            ReadHalf,
            WriteHalf,
        },
        select,
        sync::mpsc,
        time::sleep,
    },
    tokio_serial::{
        SerialPort as _,
        SerialPortBuilderExt as _,
        SerialStream,
    },
    multiworld::Filename,
    crate::{
        FrontendWriter,
        Message,
        frontend,
    },
};
#[cfg(unix)] use std::{
    ffi::OsString,
    path::Path,
};

const TEST_TIMEOUT: Duration = Duration::from_millis(200); // 200ms in the sample code
const REGULAR_TIMEOUT: Duration = Duration::from_secs(10); // twice the ping interval

const PROTOCOL_VERSION: u8 = 1;

#[derive(Debug)]
struct HandshakeResponse {
    player_id: NonZeroU8,
    file_hash: [HashIcon; 5],
    port: SerialStream,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ConnectError {
    #[error(transparent)] Io(#[from] io::Error),
    #[error(transparent)] SerialPort(#[from] tokio_serial::Error),
    #[error("failed to decode hash icon")]
    HashIcon,
    #[error("connected to EverDrive main menu")]
    MainMenu,
    #[cfg(unix)]
    #[error("non-UTF-8 string: {}", .0.to_string_lossy())]
    OsString(OsString),
    #[error("N64 reported as world 0")]
    PlayerId,
    #[cfg(unix)]
    #[error("found USB port at file system root")]
    PortAtRoot,
    #[error("unexpected handshake reply: {0:x?}")]
    UnknownReply([u8; 4]),
}

#[cfg(unix)]
impl From<OsString> for ConnectError {
    fn from(s: OsString) -> Self {
        Self::OsString(s)
    }
}

async fn connect_to_port(port_info: &tokio_serial::SerialPortInfo, log: bool) -> Result<HandshakeResponse, ConnectError> {
    #[cfg(unix)] let port_path = Path::new("/dev").join(Path::new(&port_info.port_name).file_name().ok_or(ConnectError::PortAtRoot)?).into_os_string().into_string()?;
    #[cfg(windows)] let port_path = &port_info.port_name;
    if log {
        let _ = lock!(log = crate::LOG; writeln!(&*log, "{} EverDrive: opening port at {port_path:?}", Utc::now().format("%Y-%m-%d %H:%M:%S")));
    }
    let mut port = tokio_serial::new(port_path, 9_600).timeout(TEST_TIMEOUT).open_native_async()?;
    if log {
        let _ = lock!(log = crate::LOG; writeln!(&*log, "{} EverDrive: sending cmdt to {port:?}", Utc::now().format("%Y-%m-%d %H:%M:%S")));
    }
    AsyncWriteExt::write_all(&mut port, b"cmdt\0\0\0\0\0\0\0\0\0\0\0\0").await?;
    AsyncWriteExt::flush(&mut port).await?;
    if log {
        let _ = lock!(log = crate::LOG; writeln!(&*log, "{} EverDrive: reading from {port:?}", Utc::now().format("%Y-%m-%d %H:%M:%S")));
    }
    let mut cmd = [0; 16];
    AsyncReadExt::read_exact(&mut port, &mut cmd).await?;
    match cmd {
        [b'O', b'o', b'T', b'R', PROTOCOL_VERSION, _, _, _, _, _, player_id, hash1, hash2, hash3, hash4, hash5] => {
            if log {
                let _ = lock!(log = crate::LOG; writeln!(&*log, "{} EverDrive: port is in game", Utc::now().format("%Y-%m-%d %H:%M:%S")));
            }
            port.set_timeout(REGULAR_TIMEOUT)?;
            let mut buf = [0; 16];
            buf[0] = b'M';
            buf[1] = b'W';
            buf[2] = PROTOCOL_VERSION;
            buf[3] = 1; // enable MW_SEND_OWN_ITEMS
            buf[4] = 1; // enable MW_PROGRESSIVE_ITEMS_ENABLE
            AsyncWriteExt::write_all(&mut port, &buf).await?;
            Ok(HandshakeResponse {
                player_id: NonZeroU8::new(player_id).ok_or(ConnectError::PlayerId)?,
                file_hash: [
                    all().nth(hash1.into()).ok_or(ConnectError::HashIcon)?,
                    all().nth(hash2.into()).ok_or(ConnectError::HashIcon)?,
                    all().nth(hash3.into()).ok_or(ConnectError::HashIcon)?,
                    all().nth(hash4.into()).ok_or(ConnectError::HashIcon)?,
                    all().nth(hash5.into()).ok_or(ConnectError::HashIcon)?,
                ],
                port,
            })
        }
        [b'c', b'm', b'd', b'r', ..] => {
            if log {
                let _ = lock!(log = crate::LOG; writeln!(&*log, "{} EverDrive: port is in main menu (cmdr)", Utc::now().format("%Y-%m-%d %H:%M:%S")));
            }
            Err(ConnectError::MainMenu)
        }
        [b'c', b'm', b'd', b'k', ..] => {
            if log {
                let _ = lock!(log = crate::LOG; writeln!(&*log, "{} EverDrive: port is in main menu (cmdk)", Utc::now().format("%Y-%m-%d %H:%M:%S")));
            }
            Err(ConnectError::MainMenu) // older versions of EverDrive OS
        }
        _ => {
            if log {
                let _ = lock!(log = crate::LOG; writeln!(&*log, "{} EverDrive: unknown reply from port: {:?}", Utc::now().format("%Y-%m-%d %H:%M:%S"), array_ref![cmd, 0, 4]));
            }
            Err(ConnectError::UnknownReply(*array_ref![cmd, 0, 4]))
        }
    }
}

async fn read_from_port(mut port: ReadHalf<SerialStream>) -> io::Result<(ReadHalf<SerialStream>, [u8; 16])> {
    let mut buf = [0; 16];
    port.read_exact(&mut buf).await?;
    Ok((port, buf))
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error(transparent)] Io(#[from] io::Error),
    #[error(transparent)] SerialPort(#[from] tokio_serial::Error),
    #[error(transparent)] Task(#[from] tokio::task::JoinError),
    #[error("received unknown message {0} from EverDrive")]
    UnknownCommand(u8),
    #[error("received item for world 0")]
    PlayerId,
}

pub(crate) struct Subscription {
    pub(crate) log: bool,
}

impl Recipe for Subscription {
    type Output = Message;

    fn hash(&self, state: &mut iced::advanced::Hasher) {
        TypeId::of::<Self>().hash(state);
    }

    fn stream(self: Box<Self>, _: EventStream) -> Pin<Box<dyn Stream<Item = Message> + Send>> {
        enum SubscriptionState {
            Init {
                is_retry: bool,
            },
            Connected {
                session: SessionState,
                read: Pin<Box<dyn Future<Output = io::Result<(ReadHalf<SerialStream>, [u8; 16])>> + Send>>,
                writer: WriteHalf<SerialStream>,
                rx: mpsc::Receiver<frontend::ServerMessage>,
                player_data: HashMap<NonZeroU8, (Filename, u32)>,
                queue: Vec<u16>,
            },
        }

        enum SessionState {
            Handshake,
            FileSelect,
            InGame {
                internal_count: u16,
                item_pending: bool,
            },
        }

        let log = self.log;
        stream::try_unfold(SubscriptionState::Init { is_retry: false }, move |state| async move {
            let (messages, new_state) = match state {
                SubscriptionState::Init { is_retry } => {
                    if is_retry {
                        if log {
                            let _ = lock!(log = crate::LOG; writeln!(&*log, "{} EverDrive: waiting 5 seconds before next scan", Utc::now().format("%Y-%m-%d %H:%M:%S")));
                        }
                        sleep(Duration::from_secs(5)).await;
                    }
                    let mut response = None;
                    let mut errors = Vec::default();
                    for port_info in tokio_serial::available_ports()? {
                        if log {
                            let _ = lock!(log = crate::LOG; writeln!(&*log, "{} EverDrive: attempting to connect to {port_info:?}", Utc::now().format("%Y-%m-%d %H:%M:%S")));
                        }
                        match connect_to_port(&port_info, log).await {
                            Ok(resp) => {
                                if log {
                                    let _ = lock!(log = crate::LOG; writeln!(&*log, "{} EverDrive: connection successful: {resp:?}", Utc::now().format("%Y-%m-%d %H:%M:%S")));
                                }
                                response = Some(resp);
                                break
                            }
                            Err(e) => {
                                if log {
                                    let _ = lock!(log = crate::LOG; writeln!(&*log, "{} EverDrive: connection failed: {e:?}", Utc::now().format("%Y-%m-%d %H:%M:%S")));
                                }
                                errors.push((port_info, e));
                            }
                        }
                    }
                    if let Some(HandshakeResponse { port, player_id, file_hash }) = response {
                        let (reader, writer) = io::split(port);
                        let (tx, rx) = mpsc::channel(1_024);
                        (vec![
                            Message::FrontendConnected(FrontendWriter::Mpsc(tx)),
                            Message::Plugin(Box::new(frontend::ClientMessage::PlayerId(player_id))),
                            Message::Plugin(Box::new(frontend::ClientMessage::FileHash(file_hash))),
                        ], SubscriptionState::Connected {
                            session: SessionState::Handshake,
                            read: read_from_port(reader).boxed(),
                            player_data: HashMap::default(),
                            queue: Vec::default(),
                            writer, rx,
                        })
                    } else {
                        (vec![Message::EverDriveScanFailed(Arc::new(errors))], SubscriptionState::Init { is_retry: true })
                    }
                }
                SubscriptionState::Connected { mut session, mut read, mut writer, mut rx, mut player_data, mut queue } => {
                    async fn send_player_data(port: &mut WriteHalf<SerialStream>, world: NonZeroU8, name: Filename, progressive_items: u32) -> io::Result<()> {
                        let mut buf = [0; 16];
                        buf[0] = 0x01; // Player Data
                        buf[1] = world.get();
                        *array_mut_ref![buf, 2, 8] = name.0;
                        *array_mut_ref![buf, 10, 4] = progressive_items.to_be_bytes();
                        port.write_all(&buf).await?;
                        Ok(())
                    }

                    async fn get_item(port: &mut WriteHalf<SerialStream>, queue: &[u16], internal_count: &mut u16) -> io::Result<bool> {
                        Ok(if let Some(item) = queue.get(usize::from(*internal_count)) {
                            let mut buf = [0; 16];
                            buf[0] = 0x02; // Get Item
                            *array_mut_ref![buf, 1, 2] = item.to_be_bytes();
                            port.write_all(&buf).await?;
                            *internal_count += 1;
                            true
                        } else {
                            false
                        })

                    }

                    select! {
                        Some(msg) = rx.recv() => {
                            match msg {
                                frontend::ServerMessage::ItemQueue(new_queue) => {
                                    queue = new_queue;
                                    if let SessionState::InGame { ref mut internal_count, ref mut item_pending } = session {
                                        if !*item_pending && get_item(&mut writer, &queue, internal_count).await? {
                                            *item_pending = true;
                                        }
                                    }
                                }
                                frontend::ServerMessage::GetItem(item_id) => {
                                    queue.push(item_id);
                                    if let SessionState::InGame { ref mut internal_count, ref mut item_pending } = session {
                                        if !*item_pending && get_item(&mut writer, &queue, internal_count).await? {
                                            *item_pending = true;
                                        }
                                    }
                                }
                                frontend::ServerMessage::PlayerName(world, new_name) => {
                                    let (name, progressive_items) = player_data.entry(world).or_default();
                                    *name = new_name;
                                    send_player_data(&mut writer, world, *name, *progressive_items).await?;
                                }
                                frontend::ServerMessage::ProgressiveItems(world, new_progressive_items) => {
                                    let (name, progressive_items) = player_data.entry(world).or_default();
                                    *progressive_items = new_progressive_items;
                                    send_player_data(&mut writer, world, *name, *progressive_items).await?;
                                }
                            }
                            (Vec::default(), SubscriptionState::Connected { session, read, writer, rx, player_data, queue })
                        }
                        res = &mut read => match res {
                            Ok((reader, buf)) => (
                                match buf[0] {
                                    0x00 => Vec::default(), // Ping
                                    0x01 => { // State: File Select
                                        session = SessionState::FileSelect;
                                        vec![Message::Plugin(Box::new(frontend::ClientMessage::PlayerName(Filename(*array_ref![buf, 1, 8]))))]
                                    }
                                    0x02 => { // State: In Game
                                        let mut internal_count = u16::from_be_bytes(*array_ref![buf, 1, 2]);
                                        let item_pending = if let SessionState::InGame { item_pending, .. } = session {
                                            item_pending
                                        } else {
                                            for (world, (name, progressive_items)) in mem::take(&mut player_data) {
                                                send_player_data(&mut writer, world, name, progressive_items).await?;
                                            }
                                            get_item(&mut writer, &queue, &mut internal_count).await?
                                        };
                                        session = SessionState::InGame { internal_count, item_pending };
                                        Vec::default()
                                    }
                                    0x03 => vec![Message::Plugin(Box::new(frontend::ClientMessage::SendItem {
                                        key: u64::from_be_bytes(*array_ref![buf, 1, 8]),
                                        kind: u16::from_be_bytes(*array_ref![buf, 9, 2]),
                                        target_world: NonZeroU8::new(buf[11]).ok_or(Error::PlayerId)?,
                                    }))],
                                    0x04 => { // Item Received
                                        if let SessionState::InGame { ref mut internal_count, ref mut item_pending } = session {
                                            if !get_item(&mut writer, &queue, internal_count).await? {
                                                *item_pending = false;
                                            }
                                        }
                                        Vec::default()
                                    }
                                    cmd => return Err(Error::UnknownCommand(cmd)),
                                },
                                SubscriptionState::Connected {
                                    read: read_from_port(reader).boxed(),
                                    session, writer, rx, player_data, queue,
                                },
                            ),
                            Err(e) => match e.kind() {
                                io::ErrorKind::TimedOut => (
                                    vec![Message::EverDriveTimeout],
                                    SubscriptionState::Init { is_retry: true },
                                ),
                                _ => return Err(e.into()),
                            },
                        },
                    }
                }
            };
            Ok::<_, Error>(Some((stream::iter(messages).map(Ok::<_, Error>), new_state)))
        }).try_flatten().map(|res| res.unwrap_or_else(|e| Message::FrontendSubscriptionError(Arc::new(e.into())))).boxed()
    }
}
