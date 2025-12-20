use {
    std::{
        any::TypeId,
        cmp::Ordering::*,
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
    num_traits::FromPrimitive as _,
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
    tokio_io_timeout::TimeoutStream,
    tokio_serial::{
        SerialPortBuilderExt as _,
        SerialStream,
    },
    multiworld::{
        Filename,
        HintArea,
        OptHintArea,
    },
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
    port: Pin<Box<TimeoutStream<SerialStream>>>,
    version: ootr_utils::Version,
    player_id: NonZeroU8,
    file_hash: [HashIcon; 5],
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ConnectError {
    #[error(transparent)] Io(#[from] io::Error),
    #[error(transparent)] SerialPort(#[from] tokio_serial::Error),
    #[error("unknown branch identifier: 0x{0:02x}")]
    Branch(u8),
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
    let mut port = TimeoutStream::new(tokio_serial::new(port_path, 9_600).open_native_async()?);
    port.set_read_timeout(Some(TEST_TIMEOUT));
    port.set_write_timeout(Some(TEST_TIMEOUT));
    let mut port = Box::pin(port);
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
        [b'O', b'o', b'T', b'R', PROTOCOL_VERSION, major, minor, patch, branch, supplementary, player_id, hash1, hash2, hash3, hash4, hash5] => {
            if log {
                let _ = lock!(log = crate::LOG; writeln!(&*log, "{} EverDrive: port is in game", Utc::now().format("%Y-%m-%d %H:%M:%S")));
            }
            port.as_mut().set_read_timeout_pinned(Some(REGULAR_TIMEOUT));
            port.as_mut().set_write_timeout_pinned(Some(REGULAR_TIMEOUT));
            let mut buf = [0; 16];
            buf[0] = b'M';
            buf[1] = b'W';
            buf[2] = PROTOCOL_VERSION;
            buf[3] = 1; // enable MW_SEND_OWN_ITEMS
            buf[4] = 1; // enable MW_PROGRESSIVE_ITEMS_ENABLE
            AsyncWriteExt::write_all(&mut port, &buf).await?;
            Ok(HandshakeResponse {
                version: ootr_utils::Version::from_bytes([major, minor, patch, branch, supplementary]).ok_or_else(|| ConnectError::Branch(branch))?,
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

async fn read_from_port(mut port: ReadHalf<Pin<Box<TimeoutStream<SerialStream>>>>) -> io::Result<(ReadHalf<Pin<Box<TimeoutStream<SerialStream>>>>, [u8; 16])> {
    let mut buf = [0; 16];
    port.read_exact(&mut buf).await?;
    Ok((port, buf))
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error(transparent)] Io(#[from] io::Error),
    #[error(transparent)] SerialPort(#[from] tokio_serial::Error),
    #[error(transparent)] Task(#[from] tokio::task::JoinError),
    #[error("received save data segment with out-of-range index")]
    SaveDataSegment(u8),
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

    fn hash(&self, state: &mut iced::advanced::subscription::Hasher) {
        TypeId::of::<Self>().hash(state);
    }

    fn stream(self: Box<Self>, _: EventStream) -> Pin<Box<dyn Stream<Item = Message> + Send>> {
        enum SubscriptionState {
            Init {
                is_retry: bool,
            },
            Connected {
                session: SessionState,
                read: Pin<Box<dyn Future<Output = io::Result<(ReadHalf<Pin<Box<TimeoutStream<SerialStream>>>>, [u8; 16])>> + Send>>,
                writer: WriteHalf<Pin<Box<TimeoutStream<SerialStream>>>>,
                rx: mpsc::Receiver<frontend::ServerMessage>,
                version: ootr_utils::Version,
                player_data: HashMap<NonZeroU8, (Filename, u32)>,
                save_data: [u8; oottracker::save::SIZE],
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
                    if let Some(HandshakeResponse { port, version, player_id, file_hash }) = response {
                        let (reader, writer) = io::split(port);
                        let (tx, rx) = mpsc::channel(1_024);
                        (vec![
                            Message::FrontendConnected(FrontendWriter::Mpsc(tx)),
                            Message::Plugin(Box::new(frontend::ClientMessage::PlayerId(player_id))),
                            Message::Plugin(Box::new(frontend::ClientMessage::FileHash(Some(file_hash)))),
                        ], SubscriptionState::Connected {
                            session: SessionState::Handshake,
                            read: read_from_port(reader).boxed(),
                            player_data: HashMap::default(),
                            save_data: [0; oottracker::save::SIZE],
                            queue: Vec::default(),
                            writer, rx, version,
                        })
                    } else {
                        (vec![Message::EverDriveScanFailed(Arc::new(errors))], SubscriptionState::Init { is_retry: true })
                    }
                }
                SubscriptionState::Connected { mut session, mut read, mut writer, mut rx, version, mut player_data, mut save_data, mut queue } => {
                    async fn send_player_data(port: &mut WriteHalf<Pin<Box<TimeoutStream<SerialStream>>>>, world: NonZeroU8, name: Filename, progressive_items: u32) -> io::Result<()> {
                        let mut buf = [0; 16];
                        buf[0] = 0x01; // Player Data
                        buf[1] = world.get();
                        *array_mut_ref![buf, 2, 8] = name.0;
                        *array_mut_ref![buf, 10, 4] = progressive_items.to_be_bytes();
                        port.write_all(&buf).await?;
                        Ok(())
                    }

                    async fn get_item(port: &mut WriteHalf<Pin<Box<TimeoutStream<SerialStream>>>>, queue: &[u16], internal_count: &mut u16) -> io::Result<bool> {
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
                            (Vec::default(), SubscriptionState::Connected { session, read, writer, rx, version, player_data, save_data, queue })
                        }
                        res = &mut read => match res {
                            Ok((mut reader, buf)) => (
                                match buf[0] {
                                    0x00 => Vec::default(), // Ping
                                    0x01 => { // State: File Select
                                        session = SessionState::FileSelect;
                                        vec![Message::Plugin(Box::new(frontend::ClientMessage::PlayerName(Filename(*array_ref![buf, 1, 8]))))]
                                    }
                                    0x02 => { // State: In Game
                                        if version.branch() == ootr_utils::Branch::DevFenhl && version.base().cmp(&semver::Version::new(8, 3, 68)).then_with(|| version.supplementary().cmp(&Some(2))) == Less {
                                            // older iteration of this packet without the full save data
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
                                        } else {
                                            *array_mut_ref![save_data, 0, 15] = *array_ref![buf, 1, 15];
                                            reader.read_exact(array_mut_ref![save_data, 15, 200 - 15]).await?;
                                        }
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
                                    0x05 => { // Dungeon Reward Locations
                                        let mut rest = [0; 3];
                                        reader.read_exact(&mut rest).await?;
                                        let [_, emerald_world, emerald_area, ruby_world, ruby_area, sapphire_world, sapphire_area, light_world, light_area, forest_world, forest_area, fire_world, fire_area, water_world, water_area, shadow_world] = buf;
                                        let [shadow_area, spirit_world, spirit_area] = rest;
                                        vec![Message::Plugin(Box::new(frontend::ClientMessage::DungeonRewardInfo {
                                            emerald: if let (Some(world), Some(area)) = (NonZeroU8::new(emerald_world), OptHintArea::from_u8(emerald_area).and_then(|area| HintArea::try_from(area).ok())) { Some((world, area)) } else { None },
                                            ruby: if let (Some(world), Some(area)) = (NonZeroU8::new(ruby_world), OptHintArea::from_u8(ruby_area).and_then(|area| HintArea::try_from(area).ok())) { Some((world, area)) } else { None },
                                            sapphire: if let (Some(world), Some(area)) = (NonZeroU8::new(sapphire_world), OptHintArea::from_u8(sapphire_area).and_then(|area| HintArea::try_from(area).ok())) { Some((world, area)) } else { None },
                                            light: if let (Some(world), Some(area)) = (NonZeroU8::new(light_world), OptHintArea::from_u8(light_area).and_then(|area| HintArea::try_from(area).ok())) { Some((world, area)) } else { None },
                                            forest: if let (Some(world), Some(area)) = (NonZeroU8::new(forest_world), OptHintArea::from_u8(forest_area).and_then(|area| HintArea::try_from(area).ok())) { Some((world, area)) } else { None },
                                            fire: if let (Some(world), Some(area)) = (NonZeroU8::new(fire_world), OptHintArea::from_u8(fire_area).and_then(|area| HintArea::try_from(area).ok())) { Some((world, area)) } else { None },
                                            water: if let (Some(world), Some(area)) = (NonZeroU8::new(water_world), OptHintArea::from_u8(water_area).and_then(|area| HintArea::try_from(area).ok())) { Some((world, area)) } else { None },
                                            shadow: if let (Some(world), Some(area)) = (NonZeroU8::new(shadow_world), OptHintArea::from_u8(shadow_area).and_then(|area| HintArea::try_from(area).ok())) { Some((world, area)) } else { None },
                                            spirit: if let (Some(world), Some(area)) = (NonZeroU8::new(spirit_world), OptHintArea::from_u8(spirit_area).and_then(|area| HintArea::try_from(area).ok())) { Some((world, area)) } else { None },
                                        }))]
                                    }
                                    0x06 => { // Save Data Segment
                                        let segment_idx = buf[1];
                                        if segment_idx >= 10 { return Err(Error::SaveDataSegment(segment_idx)) }
                                        *array_mut_ref![save_data, 500 * usize::from(segment_idx) + 200, 14] = *array_ref![buf, 2, 14];
                                        reader.read_exact(array_mut_ref![save_data, 500 * usize::from(segment_idx) + 214, 500 - 14]).await?;
                                        if segment_idx == 9 {
                                            let mut internal_count = u16::from_be_bytes(*array_ref![save_data, 0x90, 2]);
                                            let item_pending = if let SessionState::InGame { item_pending, .. } = session {
                                                item_pending
                                            } else {
                                                for (world, (name, progressive_items)) in mem::take(&mut player_data) {
                                                    send_player_data(&mut writer, world, name, progressive_items).await?;
                                                }
                                                get_item(&mut writer, &queue, &mut internal_count).await?
                                            };
                                            session = SessionState::InGame { internal_count, item_pending };
                                        }
                                        vec![Message::Plugin(Box::new(frontend::ClientMessage::SaveData(save_data)))]
                                    }
                                    cmd => return Err(Error::UnknownCommand(cmd)),
                                },
                                SubscriptionState::Connected {
                                    read: read_from_port(reader).boxed(),
                                    session, writer, rx, version, player_data, save_data, queue,
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
