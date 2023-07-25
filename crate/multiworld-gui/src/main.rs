#![deny(rust_2018_idioms, unused, unused_crate_dependencies, unused_import_braces, unused_lifetimes, warnings)]
#![forbid(unsafe_code)]

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use {
    std::{
        borrow::Cow,
        env,
        fmt,
        future::Future,
        io::prelude::*,
        mem,
        num::NonZeroU8,
        path::{
            Path,
            PathBuf,
        },
        process,
        sync::Arc,
        time::Duration,
    },
    async_proto::Protocol,
    chrono::prelude::*,
    dark_light::Mode::*,
    futures::{
        future,
        stream::Stream,
    },
    iced::{
        Application,
        Command,
        Element,
        Length,
        Settings,
        Subscription,
        Theme,
        clipboard,
        widget::*,
        window::{
            self,
            icon,
        },
    },
    ::image::ImageFormat,
    itertools::Itertools as _,
    once_cell::sync::Lazy,
    ootr_utils::spoiler::HashIcon,
    open::that as open,
    rfd::AsyncFileDialog,
    semver::Version,
    serenity::utils::MessageBuilder,
    sysinfo::Pid,
    tokio::{
        io,
        net::tcp::{
            OwnedReadHalf,
            OwnedWriteHalf,
        },
        sync::Mutex,
        time::{
            Instant,
            sleep_until,
        },
    },
    tokio_tungstenite::tungstenite,
    url::Url,
    wheel::{
        fs,
        traits::IsNetworkError,
    },
    multiworld::{
        DurationFormatter,
        Filename,
        LobbyView,
        RoomFormatter,
        RoomView,
        SessionState,
        SessionStateError,
        config::CONFIG,
        format_room_state,
        frontend,
        github::Repo,
        ws::{
            ServerError,
            latest::{
                ClientMessage,
                ServerMessage,
            },
        },
    },
    crate::subscriptions::WsSink,
};
#[cfg(unix)] use {
    std::os::unix::fs::PermissionsExt as _,
    xdg::BaseDirectories,
};
#[cfg(windows)] use directories::ProjectDirs;

mod login;
mod subscriptions;

static LOG: Lazy<Mutex<std::fs::File>> = Lazy::new(|| {
    let path = {
        #[cfg(unix)] {
            BaseDirectories::new().expect("failed to determine XDG base directories").place_data_file("midos-house/multiworld-gui.log").expect("failed to create log dir")
        }
        #[cfg(windows)] {
            let project_dirs = ProjectDirs::from("net", "Fenhl", "OoTR Multiworld").expect("failed to determine project directories");
            std::fs::create_dir_all(project_dirs.data_dir()).expect("failed to create log dir");
            project_dirs.data_dir().join("gui.log")
        }
    };
    Mutex::new(std::fs::File::create(path).expect("failed to create log file"))
});

struct LoggingReader {
    log: bool,
    context: &'static str,
    inner: OwnedReadHalf,
}

impl LoggingReader {
    async fn read<T: Protocol + fmt::Debug>(&mut self) -> Result<T, async_proto::ReadError> {
        let msg = T::read(&mut self.inner).await?;
        if self.log {
            writeln!(&*LOG.lock().await, "{} {}: {msg:?}", Utc::now().format("%Y-%m-%d %H:%M:%S"), self.context)?;
        }
        Ok(msg)
    }
}

struct LoggingStream<R> {
    log: bool,
    context: &'static str,
    inner: R,
}

impl<R: Stream<Item = Result<tungstenite::Message, tungstenite::Error>> + Unpin + Send> LoggingStream<R> {
    async fn read_owned(mut self) -> Result<(Self, ServerMessage), async_proto::ReadError> {
        let msg = ServerMessage::read_ws(&mut self.inner).await?;
        if self.log {
            writeln!(&*LOG.lock().await, "{} {}: {msg:?}", Utc::now().format("%Y-%m-%d %H:%M:%S"), self.context)?;
        }
        Ok((self, msg))
    }
}

#[derive(Clone)]
struct LoggingWriter {
    log: bool,
    context: &'static str,
    inner: Arc<Mutex<OwnedWriteHalf>>,
}

impl LoggingWriter {
    async fn write(&self, msg: impl Protocol + fmt::Debug) -> Result<(), async_proto::WriteError> {
        if self.log {
            writeln!(&*LOG.lock().await, "{} {}: {msg:?}", Utc::now().format("%Y-%m-%d %H:%M:%S"), self.context)?;
        }
        msg.write(&mut *self.inner.lock().await).await
    }
}

#[derive(Clone)]
struct LoggingSink {
    log: bool,
    context: &'static str,
    inner: Arc<Mutex<WsSink>>,
}

impl LoggingSink {
    async fn write(&self, msg: ClientMessage) -> Result<(), async_proto::WriteError> {
        if self.log {
            writeln!(&*LOG.lock().await, "{} {}: {msg:?}", Utc::now().format("%Y-%m-%d %H:%M:%S"), self.context)?;
        }
        msg.write_ws(&mut *self.inner.lock().await).await
    }
}

fn hash_icon(icon: HashIcon) -> Image {
    Image::new(image::Handle::from_memory(match icon {
        HashIcon::Beans => &include_bytes!("../../../assets/hash-icon/deku-stick.png")[..],
        HashIcon::BigMagic => &include_bytes!("../../../assets/hash-icon/deku-nut.png")[..],
        HashIcon::Bombchu => &include_bytes!("../../../assets/hash-icon/bow.png")[..],
        HashIcon::Boomerang => &include_bytes!("../../../assets/hash-icon/slingshot.png")[..],
        HashIcon::BossKey => &include_bytes!("../../../assets/hash-icon/fairy-ocarina.png")[..],
        HashIcon::BottledFish => &include_bytes!("../../../assets/hash-icon/bombchu.png")[..],
        HashIcon::BottledMilk => &include_bytes!("../../../assets/hash-icon/longshot.png")[..],
        HashIcon::Bow => &include_bytes!("../../../assets/hash-icon/boomerang.png")[..],
        HashIcon::Compass => &include_bytes!("../../../assets/hash-icon/lens-of-truth.png")[..],
        HashIcon::Cucco => &include_bytes!("../../../assets/hash-icon/beans.png")[..],
        HashIcon::DekuNut => &include_bytes!("../../../assets/hash-icon/megaton-hammer.png")[..],
        HashIcon::DekuStick => &include_bytes!("../../../assets/hash-icon/bottled-fish.png")[..],
        HashIcon::FairyOcarina => &include_bytes!("../../../assets/hash-icon/bottled-milk.png")[..],
        HashIcon::Frog => &include_bytes!("../../../assets/hash-icon/mask-of-truth.png")[..],
        HashIcon::GoldScale => &include_bytes!("../../../assets/hash-icon/sold-out.png")[..],
        HashIcon::HeartContainer => &include_bytes!("../../../assets/hash-icon/cucco.png")[..],
        HashIcon::HoverBoots => &include_bytes!("../../../assets/hash-icon/mushroom.png")[..],
        HashIcon::KokiriTunic => &include_bytes!("../../../assets/hash-icon/saw.png")[..],
        HashIcon::LensOfTruth => &include_bytes!("../../../assets/hash-icon/frog.png")[..],
        HashIcon::Longshot => &include_bytes!("../../../assets/hash-icon/master-sword.png")[..],
        HashIcon::Map => &include_bytes!("../../../assets/hash-icon/mirror-shield.png")[..],
        HashIcon::MaskOfTruth => &include_bytes!("../../../assets/hash-icon/kokiri-tunic.png")[..],
        HashIcon::MasterSword => &include_bytes!("../../../assets/hash-icon/hover-boots.png")[..],
        HashIcon::MegatonHammer => &include_bytes!("../../../assets/hash-icon/silver-gauntlets.png")[..],
        HashIcon::MirrorShield => &include_bytes!("../../../assets/hash-icon/gold-scale.png")[..],
        HashIcon::Mushroom => &include_bytes!("../../../assets/hash-icon/stone-of-agony.png")[..],
        HashIcon::Saw => &include_bytes!("../../../assets/hash-icon/skull-token.png")[..],
        HashIcon::SilverGauntlets => &include_bytes!("../../../assets/hash-icon/heart-container.png")[..],
        HashIcon::SkullToken => &include_bytes!("../../../assets/hash-icon/boss-key.png")[..],
        HashIcon::Slingshot => &include_bytes!("../../../assets/hash-icon/compass.png")[..],
        HashIcon::SoldOut => &include_bytes!("../../../assets/hash-icon/map.png")[..],
        HashIcon::StoneOfAgony => &include_bytes!("../../../assets/hash-icon/big-magic.png")[..],
    }))
}

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)] Client(#[from] multiworld::ClientError),
    #[error(transparent)] Elapsed(#[from] tokio::time::error::Elapsed),
    #[error(transparent)] Io(#[from] io::Error),
    #[error(transparent)] Json(#[from] serde_json::Error),
    #[error(transparent)] Read(#[from] async_proto::ReadError),
    #[error(transparent)] Reqwest(#[from] reqwest::Error),
    #[error(transparent)] Semver(#[from] semver::Error),
    #[error(transparent)] Url(#[from] url::ParseError),
    #[error(transparent)] WebSocket(#[from] tungstenite::Error),
    #[error(transparent)] Wheel(#[from] wheel::Error),
    #[error(transparent)] Write(#[from] async_proto::WriteError),
    #[cfg(unix)] #[error(transparent)] Xdg(#[from] xdg::BaseDirectoriesError),
    #[error("tried to copy debug info with no active error")]
    CopyDebugInfo,
    #[cfg(windows)]
    #[error("user folder not found")]
    MissingHomeDir,
    #[error("protocol version mismatch: {frontend} plugin is version {version} but we're version {}", frontend::PROTOCOL_VERSION)]
    VersionMismatch {
        frontend: FrontendFlags,
        version: u8,
    },
}

impl IsNetworkError for Error {
    fn is_network_error(&self) -> bool {
        match self {
            Self::Client(e) => e.is_network_error(),
            Self::Elapsed(_) => true,
            Self::Io(e) => e.is_network_error(),
            Self::Read(e) => e.is_network_error(),
            Self::Reqwest(e) => e.is_network_error(),
            Self::Write(e) => e.is_network_error(),
            _ => false,
        }
    }
}

#[derive(Debug, Clone)]
enum Message {
    CommandError(Arc<Error>),
    ConfirmRoomDeletion,
    CopyDebugInfo,
    DiscordChannel,
    DiscordInvite,
    DismissWrongPassword,
    Exit,
    FrontendConnected(Arc<Mutex<OwnedWriteHalf>>),
    FrontendSubscriptionError(Arc<Error>),
    JoinRoom,
    Kick(NonZeroU8),
    LoginError(Arc<login::Error>),
    LoginToken(String),
    NewIssue,
    Nop,
    OpenLoginPage(Url),
    Plugin(Box<frontend::ClientMessage>), // boxed due to the large size of save data; if Message is too large, iced will overflow the stack on window resize
    ReconnectFrontend,
    ReconnectToLobby,
    ReconnectToRoom(u64, String),
    SendAll,
    SendAllBrowse,
    Server(ServerMessage),
    ServerConnected(Arc<Mutex<WsSink>>),
    ServerSubscriptionError(Arc<Error>),
    SetAutoDeleteDelta(DurationFormatter),
    SetCreateNewRoom(bool),
    SetExistingRoomSelection(RoomFormatter),
    SetLobbyView(LobbyView),
    SetNewRoomName(String),
    SetPassword(String),
    SetRoomView(RoomView),
    SetSendAllPath(String),
    SetSendAllWorld(String),
    UpToDate,
}

fn cmd(future: impl Future<Output = Result<Message, Error>> + Send + 'static) -> Command<Message> {
    Command::single(iced_native::command::Action::Future(Box::pin(async move {
        match future.await {
            Ok(msg) => msg,
            Err(e) => Message::CommandError(Arc::new(e.into())),
        }
    })))
}

struct State {
    frontend: FrontendFlags,
    debug_info_copied: bool,
    command_error: Option<Arc<Error>>,
    login_error: Option<Arc<login::Error>>,
    frontend_subscription_error: Option<Arc<Error>>,
    frontend_connection_id: u8,
    frontend_writer: Option<LoggingWriter>,
    log: bool,
    last_login_url: Option<Url>,
    websocket_url: Url,
    server_connection: SessionState<Arc<Error>>,
    server_writer: Option<LoggingSink>,
    retry: Instant,
    wait_time: Duration,
    last_world: Option<NonZeroU8>,
    last_name: Filename,
    last_hash: Option<[HashIcon; 5]>,
    last_save: Option<oottracker::Save>,
    pending_items_before_save: Vec<(u32, u16, NonZeroU8)>,
    pending_items_after_save: Vec<(u32, u16, NonZeroU8)>,
    updates_checked: bool,
    send_all_path: String,
    send_all_world: String,
}

impl State {
    fn error_to_markdown(&self) -> Option<String> {
        Some(if let Some(ref e) = self.command_error {
            MessageBuilder::default()
                .push_line(concat!("error in Mido's House Multiworld version ", env!("CARGO_PKG_VERSION"), ":"))
                .push_line_safe(e)
                .push_codeblock_safe(format!("{e:?}"), Some("rust"))
                .build()
        } else if let Some(ref e) = self.login_error {
            let with_provider = if let SessionState::Lobby { view: LobbyView::Login(provider), .. } = self.server_connection {
                format!(" with {provider}")
            } else {
                String::default()
            };
            MessageBuilder::default()
                .push_line(format!("error in Mido's House Multiworld version {} while trying to sign in{with_provider}:", env!("CARGO_PKG_VERSION")))
                .push_line_safe(e)
                .push_codeblock_safe(format!("{e:?}"), Some("rust"))
                .build()
        } else if let Some(ref e) = self.frontend_subscription_error {
            MessageBuilder::default()
                .push_line(format!("error in Mido's House Multiworld version {} during communication with {}:", env!("CARGO_PKG_VERSION"), self.frontend.display_with_version()))
                .push_line_safe(e)
                .push_codeblock_safe(format!("{e:?}"), Some("rust"))
                .build()
        } else if let SessionState::Error { ref e, .. } = self.server_connection {
            MessageBuilder::default()
                .push_line(concat!("error in Mido's House Multiworld version ", env!("CARGO_PKG_VERSION"), " during communication with the server:"))
                .push_line_safe(e)
                .push_codeblock_safe(format!("{e:?}"), Some("rust"))
                .build()
        } else {
            return None
        })
    }
}

#[derive(Debug, Clone)]
enum FrontendFlags {
    BizHawk {
        path: PathBuf,
        pid: Pid,
        local_bizhawk_version: Version,
        port: u16,
    },
    Pj64V3,
    Pj64V4,
}

impl FrontendFlags {
    fn display_with_version(&self) -> Cow<'static, str> {
        match self {
            Self::BizHawk { local_bizhawk_version, .. } => format!("BizHawk {local_bizhawk_version}").into(),
            Self::Pj64V3 => "Project64 3.x".into(),
            Self::Pj64V4 => "Project64 4.x".into(),
        }
    }
}

impl fmt::Display for FrontendFlags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BizHawk { .. } => write!(f, "BizHawk"),
            Self::Pj64V3 | Self::Pj64V4 => write!(f, "Project64"),
        }
    }
}

impl Application for State {
    type Executor = iced::executor::Default;
    type Message = Message;
    type Theme = Theme;
    type Flags = FrontendFlags;

    fn new(frontend: FrontendFlags) -> (Self, Command<Message>) {
        (Self {
            frontend: frontend.clone(),
            debug_info_copied: false,
            command_error: None,
            login_error: None,
            frontend_subscription_error: None,
            frontend_connection_id: 0,
            frontend_writer: None,
            log: CONFIG.log,
            last_login_url: None,
            websocket_url: CONFIG.websocket_url().expect("failed to parse WebSocket URL"),
            server_connection: SessionState::Init,
            server_writer: None,
            retry: Instant::now(),
            wait_time: Duration::from_secs(1),
            last_world: None,
            last_name: Filename::default(),
            last_hash: None,
            last_save: None,
            pending_items_before_save: Vec::default(),
            pending_items_after_save: Vec::default(),
            updates_checked: false,
            send_all_path: String::default(),
            send_all_world: String::default(),
        }, cmd(async move {
            let http_client = reqwest::Client::builder()
                .user_agent(concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION")))
                .use_rustls_tls()
                .https_only(true)
                .http2_prior_knowledge()
                .build()?;
            let repo = Repo::new("midoshouse", "ootr-multiworld");
            if let Some(release) = repo.latest_release(&http_client).await? {
                let new_ver = release.version()?;
                if new_ver > Version::parse(env!("CARGO_PKG_VERSION"))? {
                    let updater_path = {
                        #[cfg(unix)] {
                            BaseDirectories::new()?.place_cache_file("midos-house/multiworld-updater")?
                        }
                        #[cfg(windows)] {
                            let project_dirs = ProjectDirs::from("net", "Fenhl", "OoTR Multiworld").ok_or(Error::MissingHomeDir)?;
                            let cache_dir = project_dirs.cache_dir();
                            fs::create_dir_all(cache_dir).await?;
                            cache_dir.join("updater.exe")
                        }
                    };
                    #[cfg(all(target_arch = "x86_64", target_os = "linux", debug_assertions))] let updater_data = include_bytes!("../../../target/debug/multiworld-updater");
                    #[cfg(all(target_arch = "x86_64", target_os = "linux", not(debug_assertions)))] let updater_data = include_bytes!("../../../target/release/multiworld-updater");
                    #[cfg(all(target_arch = "x86_64", target_os = "windows", debug_assertions))] let updater_data = include_bytes!("../../../target/debug/multiworld-updater.exe");
                    #[cfg(all(target_arch = "x86_64", target_os = "windows", not(debug_assertions)))] let updater_data = include_bytes!("../../../target/release/multiworld-updater.exe");
                    fs::write(&updater_path, updater_data).await?;
                    #[cfg(unix)] fs::set_permissions(&updater_path, fs::Permissions::from_mode(0o755)).await?;
                    let mut cmd = std::process::Command::new(updater_path);
                    match frontend {
                        FrontendFlags::BizHawk { path, pid, local_bizhawk_version, port: _ } => {
                            cmd.arg("bizhawk");
                            cmd.arg(process::id().to_string());
                            cmd.arg(path);
                            cmd.arg(pid.to_string());
                            cmd.arg(local_bizhawk_version.to_string());
                        }
                        FrontendFlags::Pj64V3 | FrontendFlags::Pj64V4 => {
                            cmd.arg("pj64");
                            cmd.arg(env::current_exe()?);
                            cmd.arg(process::id().to_string());
                        }
                    }
                    let _ = cmd.spawn()?;
                    return Ok(Message::Exit)
                }
            }
            Ok(Message::UpToDate)
        }))
    }

    fn theme(&self) -> Self::Theme {
        match dark_light::detect() { //TODO automatically update on system theme change
            Dark => Theme::Dark,
            Light | Default => Theme::Light,
        }
    }

    fn title(&self) -> String {
        format!("Mido's House Multiworld for {}", self.frontend)
    }

    fn update(&mut self, msg: Message) -> Command<Message> {
        match msg {
            Message::SetLobbyView(new_view) => if let SessionState::Lobby { ref mut view, .. } = self.server_connection {
                *view = new_view;
            },
            Message::SetRoomView(new_view) => if let SessionState::Room { ref mut view, .. } = self.server_connection {
                *view = new_view;
            },
            Message::CommandError(e) => { self.command_error.get_or_insert(e); }
            Message::ConfirmRoomDeletion => if let Some(writer) = self.server_writer.clone() {
                return cmd(async move {
                    writer.write(ClientMessage::DeleteRoom).await?;
                    Ok(Message::Nop)
                })
            },
            Message::CopyDebugInfo => if let Some(error_md) = self.error_to_markdown() {
                self.debug_info_copied = true;
                return clipboard::write(error_md)
            } else {
                return cmd(future::err(Error::CopyDebugInfo))
            },
            Message::DiscordChannel => if let Err(e) = open("https://discord.com/channels/274180765816848384/476723801032491008") {
                return cmd(future::err(e.into()))
            },
            Message::DiscordInvite => if let Err(e) = open("https://discord.gg/BGRrKKn") {
                return cmd(future::err(e.into()))
            },
            Message::DismissWrongPassword => if let SessionState::Lobby { ref mut wrong_password, .. } = self.server_connection {
                *wrong_password = false;
            },
            Message::Exit => return window::close(),
            Message::FrontendConnected(writer) => {
                let writer = LoggingWriter { log: self.log, context: "to frontend", inner: Arc::clone(&writer) };
                self.frontend_writer = Some(writer.clone());
                if let SessionState::Room { ref players, ref item_queue, .. } = self.server_connection {
                    let players = players.clone();
                    let item_queue = item_queue.clone();
                    return cmd(async move {
                        for player in players {
                            writer.write(frontend::ServerMessage::PlayerName(player.world, if player.name == Filename::default() {
                                Filename::fallback(player.world)
                            } else {
                                player.name
                            })).await?;
                        }
                        if !item_queue.is_empty() {
                            writer.write(frontend::ServerMessage::ItemQueue(item_queue)).await?;
                        }
                        Ok(Message::Nop)
                    })
                }
            }
            Message::FrontendSubscriptionError(e) => {
                if let Error::Read(async_proto::ReadError::Io(ref e)) = *e {
                    match (&self.frontend, e.kind()) {
                        (FrontendFlags::BizHawk { .. } | FrontendFlags::Pj64V4, io::ErrorKind::ConnectionReset | io::ErrorKind::UnexpectedEof) => return window::close(), // BizHawk closed
                        (FrontendFlags::Pj64V3, io::ErrorKind::ConnectionReset) => {
                            self.frontend_writer = None;
                            return Command::none()
                        }
                        (_, _) => {}
                    }
                }
                self.frontend_subscription_error.get_or_insert(e);
            }
            Message::JoinRoom => if let SessionState::Lobby { create_new_room, ref existing_room_selection, ref new_room_name, ref password, .. } = self.server_connection {
                if !password.is_empty() {
                    let existing_room_selection = existing_room_selection.clone();
                    let new_room_name = new_room_name.clone();
                    let password = password.clone();
                    let writer = self.server_writer.clone().expect("join room button only appears when connected to server");
                    return cmd(async move {
                        if create_new_room {
                            if !new_room_name.is_empty() {
                                writer.write(ClientMessage::CreateRoom { name: new_room_name, password }).await?;
                            }
                        } else {
                            if let Some(room) = existing_room_selection {
                                writer.write(ClientMessage::JoinRoom { id: room.id, password: room.password_required.then_some(password) }).await?;
                            }
                        }
                        Ok(Message::Nop)
                    })
                }
            }
            Message::Kick(player_id) => if let Some(writer) = self.server_writer.clone() {
                return cmd(async move {
                    writer.write(ClientMessage::KickPlayer(player_id)).await?;
                    Ok(Message::Nop)
                })
            },
            Message::LoginError(e) => { self.login_error.get_or_insert(e); }
            Message::LoginToken(bearer_token) => if let SessionState::Lobby { view: LobbyView::Login(provider), .. } = self.server_connection {
                if let Some(writer) = self.server_writer.clone() {
                    return cmd(async move {
                        writer.write(match provider {
                            login::Provider::RaceTime => ClientMessage::LoginRaceTime { bearer_token },
                            login::Provider::Discord => ClientMessage::LoginDiscord { bearer_token },
                        }).await?;
                        Ok(Message::SetLobbyView(LobbyView::Settings))
                    })
                }
            },
            Message::NewIssue => {
                let mut issue_url = match Url::parse("https://github.com/midoshouse/ootr-multiworld/issues/new") {
                    Ok(issue_url) => issue_url,
                    Err(e) => return cmd(future::err(e.into())),
                };
                if let Some(error_md) = self.error_to_markdown() {
                    issue_url.query_pairs_mut().append_pair("body", &error_md);
                }
                if let Err(e) = open(issue_url.to_string()) {
                    return cmd(future::err(e.into()))
                }
            }
            Message::Nop => {}
            Message::OpenLoginPage(url) => {
                self.last_login_url = Some(url.clone());
                return cmd(async move {
                    open(url.to_string())?; //TODO async
                    Ok(Message::Nop)
                })
            }
            Message::Plugin(msg) => match *msg {
                frontend::ClientMessage::PlayerId(new_player_id) => {
                    let (new_player_name, new_file_hash) = if self.last_world.replace(new_player_id).is_none() {
                        (
                            (self.last_name != Filename::default()).then_some(self.last_name),
                            self.last_hash,
                        )
                    } else {
                        (None, None)
                    };
                    if let Some(ref writer) = self.server_writer {
                        if let SessionState::Room { .. } = self.server_connection {
                            let writer = writer.clone();
                            return cmd(async move {
                                writer.write(ClientMessage::PlayerId(new_player_id)).await?;
                                if let Some(new_player_name) = new_player_name {
                                    writer.write(ClientMessage::PlayerName(new_player_name)).await?;
                                }
                                if let Some(new_file_hash) = new_file_hash {
                                    writer.write(ClientMessage::FileHash(new_file_hash)).await?;
                                }
                                Ok(Message::Nop)
                            })
                        }
                    }
                }
                frontend::ClientMessage::PlayerName(new_player_name) => {
                    self.last_name = new_player_name;
                    if self.last_world.is_some() {
                        if let Some(ref writer) = self.server_writer {
                            if let SessionState::Room { .. } = self.server_connection {
                                let writer = writer.clone();
                                return cmd(async move {
                                    writer.write(ClientMessage::PlayerName(new_player_name)).await?;
                                    Ok(Message::Nop)
                                })
                            }
                        }
                    }
                }
                frontend::ClientMessage::SendItem { key, kind, target_world } => {
                    if let Self { server_writer: Some(writer), server_connection: SessionState::Room { .. }, .. } = self {
                        let writer = writer.clone();
                        return cmd(async move {
                            writer.write(ClientMessage::SendItem { key, kind, target_world }).await?;
                            Ok(Message::Nop)
                        })
                    } else {
                        self.pending_items_after_save.push((key, kind, target_world));
                    }
                }
                frontend::ClientMessage::SaveData(save) => match oottracker::Save::from_save_data(&save) {
                    Ok(save) => {
                        self.last_save = Some(save.clone());
                        self.pending_items_before_save.extend(self.pending_items_after_save.drain(..));
                        if let Some(ref writer) = self.server_writer {
                            if let SessionState::Room { .. } = self.server_connection {
                                let writer = writer.clone();
                                return cmd(async move {
                                    writer.write(ClientMessage::SaveData(save)).await?; //TODO only send if room is marked as being tracked?
                                    Ok(Message::Nop)
                                })
                            }
                        }
                    }
                    Err(e) => if let Some(writer) = self.server_writer.clone() {
                        return cmd(async move {
                            writer.write(ClientMessage::SaveDataError { debug: format!("{e:?}"), version: multiworld::version() }).await?;
                            Ok(Message::Nop)
                        })
                    },
                },
                frontend::ClientMessage::FileHash(new_hash) => {
                    self.last_hash = Some(new_hash);
                    if self.last_world.is_some() {
                        if let Some(ref writer) = self.server_writer {
                            if let SessionState::Room { .. } = self.server_connection {
                                let writer = writer.clone();
                                return cmd(async move {
                                    writer.write(ClientMessage::FileHash(new_hash)).await?;
                                    Ok(Message::Nop)
                                })
                            }
                        }
                    }
                }
                frontend::ClientMessage::ResetPlayerId => {
                    self.last_world = None;
                    if let Some(ref writer) = self.server_writer {
                        if let SessionState::Room { .. } = self.server_connection {
                            let writer = writer.clone();
                            return cmd(async move {
                                writer.write(ClientMessage::ResetPlayerId).await?;
                                Ok(Message::Nop)
                            })
                        }
                    }
                }
            },
            Message::ReconnectFrontend => {
                self.frontend_subscription_error = None;
                self.frontend_connection_id = self.frontend_connection_id.wrapping_add(1);
            }
            Message::ReconnectToLobby => self.server_connection = SessionState::Init,
            Message::ReconnectToRoom(room_id, room_password) => self.server_connection = SessionState::InitAutoRejoin { room_id, room_password },
            Message::SendAll => {
                let server_writer = self.server_writer.clone().expect("SendAll button only appears when connected to server");
                let source_world = self.send_all_world.parse().expect("SendAll button only appears when source world is valid");
                let spoiler_log_path = Path::new(&self.send_all_path).to_owned();
                return cmd(async move {
                    let spoiler_log = serde_json::from_str(&fs::read_to_string(spoiler_log_path).await?)?;
                    server_writer.write(ClientMessage::SendAll { source_world, spoiler_log }).await?;
                    Ok(Message::Nop)
                })
            }
            Message::SendAllBrowse => return cmd(async move {
                Ok(if let Some(file) = AsyncFileDialog::new()
                    .add_filter("JSON Document", &["json"])
                    .pick_file().await
                {
                    Message::SetSendAllPath(file.path().to_str().expect("Windows paths are valid UTF-8").to_owned())
                } else {
                    Message::Nop
                })
            }),
            Message::Server(msg) => {
                let room_still_exists = if let ServerMessage::EnterLobby { ref rooms } = msg {
                    if let SessionState::InitAutoRejoin { ref room_id, .. } = self.server_connection {
                        rooms.contains_key(room_id)
                    } else {
                        false
                    }
                } else {
                    false
                };
                self.server_connection.apply(msg.clone());
                match msg {
                    ServerMessage::StructuredError(ServerError::RoomExists) => if let SessionState::Lobby { .. } = self.server_connection {
                        return cmd(future::ok(Message::JoinRoom))
                    },
                    ServerMessage::EnterLobby { .. } => {
                        let frontend_writer = self.frontend_writer.clone();
                        return cmd(async move {
                            if let Some(frontend_writer) = frontend_writer {
                                frontend_writer.write(frontend::ServerMessage::ItemQueue(Vec::default())).await?;
                            }
                            Ok(if room_still_exists { Message::JoinRoom } else { Message::Nop })
                        })
                    }
                    ServerMessage::EnterRoom { players, .. } => {
                        let server_writer = self.server_writer.clone().expect("join room button only appears when connected to server");
                        let frontend_writer = self.frontend_writer.clone().expect("join room button only appears when connected to frontend");
                        let player_id = self.last_world;
                        let player_name = self.last_name;
                        let file_hash = self.last_hash;
                        let save = self.last_save.clone();
                        let pending_items_before_save = mem::take(&mut self.pending_items_before_save);
                        let pending_items_after_save = mem::take(&mut self.pending_items_after_save);
                        return cmd(async move {
                            if let Some(player_id) = player_id {
                                server_writer.write(ClientMessage::PlayerId(player_id)).await?;
                                if player_name != Filename::default() {
                                    server_writer.write(ClientMessage::PlayerName(player_name)).await?;
                                }
                                if let Some(hash) = file_hash {
                                    server_writer.write(ClientMessage::FileHash(hash)).await?;
                                }
                            }
                            for (key, kind, target_world) in pending_items_before_save {
                                server_writer.write(ClientMessage::SendItem { key, kind, target_world }).await?;
                            }
                            if let Some(save) = save {
                                server_writer.write(ClientMessage::SaveData(save)).await?; //TODO only send if room is marked as being tracked?
                            }
                            for (key, kind, target_world) in pending_items_after_save {
                                server_writer.write(ClientMessage::SendItem { key, kind, target_world }).await?;
                            }
                            for player in players {
                                frontend_writer.write(frontend::ServerMessage::PlayerName(player.world, if player.name == Filename::default() {
                                    Filename::fallback(player.world)
                                } else {
                                    player.name
                                })).await?;
                            }
                            Ok(Message::Nop)
                        })
                    }
                    ServerMessage::PlayerName(world, name) => if let Some(writer) = self.frontend_writer.clone() {
                        return cmd(async move {
                            writer.write(frontend::ServerMessage::PlayerName(world, name)).await?;
                            Ok(Message::Nop)
                        })
                    },
                    ServerMessage::ItemQueue(queue) => if let SessionState::Room { wrong_file_hash: None, .. } = self.server_connection {
                        if let Some(writer) = self.frontend_writer.clone() {
                            return cmd(async move {
                                writer.write(frontend::ServerMessage::ItemQueue(queue)).await?;
                                Ok(Message::Nop)
                            })
                        }
                    },
                    ServerMessage::GetItem(item) => if let SessionState::Room { wrong_file_hash: None, .. } = self.server_connection {
                        if let Some(writer) = self.frontend_writer.clone() {
                            return cmd(async move {
                                writer.write(frontend::ServerMessage::GetItem(item)).await?;
                                Ok(Message::Nop)
                            })
                        }
                    },
                    ServerMessage::ProgressiveItems { world, state } => if let SessionState::Room { wrong_file_hash: None, .. } = self.server_connection {
                        if let Some(writer) = self.frontend_writer.clone() {
                            return cmd(async move {
                                writer.write(frontend::ServerMessage::ProgressiveItems(world, state)).await?;
                                Ok(Message::Nop)
                            })
                        }
                    },
                    _ => {}
                }
            }
            Message::ServerConnected(sink) => self.server_writer = Some(LoggingSink { log: self.log, context: "to server", inner: sink }),
            Message::ServerSubscriptionError(e) => if !matches!(self.server_connection, SessionState::Error { .. }) {
                if e.is_network_error() {
                    if self.retry.elapsed() >= Duration::from_secs(60 * 60 * 24) {
                        self.wait_time = Duration::from_secs(1); // reset wait time after no error for a day
                    } else {
                        self.wait_time *= 2; // exponential backoff
                    }
                    self.retry = Instant::now() + self.wait_time;
                    let retry = self.retry;
                    let reconnect_msg = if let SessionState::Room { room_id, ref room_password, .. } = self.server_connection {
                        Message::ReconnectToRoom(room_id, room_password.clone())
                    } else {
                        Message::ReconnectToLobby
                    };
                    self.server_connection = SessionState::Error {
                        e: SessionStateError::Connection(e),
                        auto_retry: true,
                    };
                    return cmd(async move {
                        sleep_until(retry).await;
                        Ok(reconnect_msg)
                    })
                } else {
                    self.server_connection = SessionState::Error {
                        e: SessionStateError::Connection(e),
                        auto_retry: false,
                    };
                }
            },
            Message::SetAutoDeleteDelta(DurationFormatter(new_delta)) => if let Some(writer) = self.server_writer.clone() {
                return cmd(async move {
                    writer.write(ClientMessage::AutoDeleteDelta(new_delta)).await?;
                    Ok(Message::Nop)
                })
            },
            Message::SetCreateNewRoom(new_val) => if let SessionState::Lobby { ref mut create_new_room, .. } = self.server_connection { *create_new_room = new_val },
            Message::SetExistingRoomSelection(name) => if let SessionState::Lobby { ref mut existing_room_selection, .. } = self.server_connection { *existing_room_selection = Some(name) },
            Message::SetNewRoomName(name) => if let SessionState::Lobby { ref mut new_room_name, .. } = self.server_connection { *new_room_name = name },
            Message::SetPassword(new_password) => if let SessionState::Lobby { ref mut password, .. } = self.server_connection { *password = new_password },
            Message::SetSendAllPath(new_path) => self.send_all_path = new_path,
            Message::SetSendAllWorld(new_world) => self.send_all_world = new_world,
            Message::UpToDate => self.updates_checked = true,
        }
        Command::none()
    }

    fn view(&self) -> Element<'_, Message> {
        if let Some(ref e) = self.command_error {
            error_view("An error occurred:", e, self.debug_info_copied)
        } else if let Some(ref e) = self.login_error {
            error_view("An error occurred while trying to sign in:", e, self.debug_info_copied) //TODO button to reset error state
        } else if let Some(ref e) = self.frontend_subscription_error {
            if let Error::Io(ref e) = **e {
                if e.kind() == io::ErrorKind::AddrInUse {
                    Column::new()
                        .push(Text::new("Connection Busy").size(24))
                        .push(Text::new(format!("Could not connect to {} because the connection is already in use. Maybe you still have another instance of this app open?", self.frontend)))
                        .push(Button::new("Retry").on_press(Message::ReconnectFrontend))
                        .spacing(8)
                        .padding(8)
                        .into()
                } else {
                    error_view(format!("An error occurred during communication with {}:", self.frontend), e, self.debug_info_copied)
                }
            } else {
                error_view(format!("An error occurred during communication with {}:", self.frontend), e, self.debug_info_copied)
            }
        } else if !self.updates_checked {
            Column::new()
                .push("Checking for updates…")
                .spacing(8)
                .padding(8)
                .into()
        } else if self.frontend_writer.is_none() {
            Column::new()
                .push(Text::new(format!("Waiting for {}…", self.frontend)))
                .push(match &self.frontend {
                    FrontendFlags::BizHawk { .. } => "Make sure your game is running and unpaused.",
                    FrontendFlags::Pj64V3 => "1. In Project64's Debugger menu, select Scripts\n2. In the Scripts window, select ootrmw.js and click Run\n3. Wait until the Output area says “Connected to multiworld app”. (This should take less than 5 seconds.) You can then close the Scripts window.",
                    FrontendFlags::Pj64V4 => "This should take less than 5 seconds.",
                })
                .spacing(8)
                .padding(8)
                .into()
        } else {
            match self.server_connection {
                SessionState::Error { auto_retry: false, ref e } => error_view("An error occurred during communication with the server:", e, self.debug_info_copied),
                SessionState::Error { auto_retry: true, ref e } => Column::new()
                    .push("A network error occurred:")
                    .push(Text::new(e.to_string()))
                    .push(Row::new()
                        .push(Button::new("Copy debug info").on_press(Message::CopyDebugInfo))
                        .push(if self.debug_info_copied { "Copied!" } else { "for pasting into Discord" })
                        .spacing(8)
                    )
                    .push(Text::new(if let Ok(retry) = chrono::Duration::from_std(self.retry.duration_since(Instant::now())) {
                        format!("Reconnecting at {}", (Local::now() + retry).format("%H:%M:%S"))
                    } else {
                        format!("Reconnecting…")
                    })) //TODO live countdown
                    .spacing(8)
                    .padding(8)
                    .into(),
                SessionState::Init => Column::new()
                    .push("Connecting to server…")
                    .push("If this takes longer than 5 seconds, check your internet connection or contact @fenhl on Discord for support.")
                    .spacing(8)
                    .padding(8)
                    .into(),
                SessionState::InitAutoRejoin { .. } => Column::new()
                    .push("Reconnecting to room…")
                    .push("If this takes longer than 5 seconds, check your internet connection or contact @fenhl on Discord for support.")
                    .spacing(8)
                    .padding(8)
                    .into(),
                SessionState::Lobby { wrong_password: true, .. } => Column::new()
                    .push("wrong password")
                    .push(Button::new("OK").on_press(Message::DismissWrongPassword))
                    .spacing(8)
                    .padding(8)
                    .into(),
                SessionState::Lobby { view: LobbyView::Settings, wrong_password: false, .. } => Column::new()
                    .push(Row::new()
                        .push(Button::new("Back").on_press(Message::SetLobbyView(LobbyView::Normal)))
                        .push(Space::with_width(Length::Fill))
                        .push(concat!("version ", env!("CARGO_PKG_VERSION")))
                    )
                    //TODO persist login state and show here, with option to sign out
                    .push(Button::new("Sign in with racetime.gg").on_press(Message::SetLobbyView(LobbyView::Login(login::Provider::RaceTime))))
                    .push(Button::new("Sign in with Discord").on_press(Message::SetLobbyView(LobbyView::Login(login::Provider::Discord))))
                    .spacing(8)
                    .padding(8)
                    .into(),
                SessionState::Lobby { view: LobbyView::Login(provider), wrong_password: false, .. } => Column::new()
                    .push(Text::new(format!("Signing in with {provider}…")))
                    .push("Please continue in your web browser.")
                    .push({
                        let mut btn = Button::new("Reopen Web Page");
                        if let Some(ref last_login_url) = self.last_login_url {
                            btn = btn.on_press(Message::OpenLoginPage(last_login_url.clone()));
                        }
                        btn
                    })
                    .push(Button::new("Cancel").on_press(Message::SetLobbyView(LobbyView::Settings)))
                    .spacing(8)
                    .padding(8)
                    .into(),
                SessionState::Lobby { view: LobbyView::Normal, wrong_password: false, ref rooms, create_new_room, ref existing_room_selection, ref new_room_name, ref password, .. } => {
                    let mut col = Column::new()
                        .push(Radio::new("Connect to existing room", false, Some(create_new_room), Message::SetCreateNewRoom))
                        .push(Radio::new("Create new room", true, Some(create_new_room), Message::SetCreateNewRoom))
                        .push(if create_new_room {
                            Element::from(TextInput::new("Room name", new_room_name).on_input(Message::SetNewRoomName).on_paste(Message::SetNewRoomName).on_submit(Message::JoinRoom).padding(5))
                        } else {
                            if rooms.is_empty() {
                                Text::new("(no rooms currently open)").into()
                            } else {
                                let mut rooms = rooms.iter().map(|(&id, (name, password_required))| RoomFormatter { id, name: name.clone(), password_required: password_required.clone() }).collect_vec();
                                rooms.sort();
                                PickList::new(rooms, existing_room_selection.clone(), Message::SetExistingRoomSelection).into()
                            }
                        });
                    if existing_room_selection.as_ref().map_or(true, |existing_room_selection| existing_room_selection.password_required) {
                        col = col.push(TextInput::new("Password", password).password().on_input(Message::SetPassword).on_paste(Message::SetPassword).on_submit(Message::JoinRoom).padding(5));
                    }
                    col
                        .push(Space::with_height(Length::Fill))
                        .push(Row::new()
                            .push({
                                let mut btn = Button::new("Connect");
                                if if create_new_room { !new_room_name.is_empty() } else { existing_room_selection.is_some() } && !password.is_empty() { btn = btn.on_press(Message::JoinRoom) }
                                btn
                            })
                            .push(Space::with_width(Length::Fill))
                            .push(Button::new("Settings").on_press(Message::SetLobbyView(LobbyView::Settings)))
                        )
                        .spacing(8)
                        .padding(8)
                        .into()
                }
                SessionState::Room { view: RoomView::ConfirmDeletion, .. } => Column::new()
                    .push("Are you sure you want to delete this room? Items that have already been sent will be lost forever!")
                    .push(Row::new()
                        .push(Button::new("Delete").on_press(Message::ConfirmRoomDeletion))
                        .push(Button::new("Back").on_press(Message::SetRoomView(RoomView::Normal)))
                        .spacing(8)
                    )
                    .spacing(8)
                    .padding(8)
                    .into(),
                SessionState::Room { wrong_file_hash: Some([[server1, server2, server3, server4, server5], [client1, client2, client3, client4, client5]]), .. } => Column::new()
                    .push("This room is for a different seed.")
                    .push(Row::new()
                        .push("Room:")
                        //TODO add gray background in light mode
                        .push(hash_icon(server1))
                        .push(hash_icon(server2))
                        .push(hash_icon(server3))
                        .push(hash_icon(server4))
                        .push(hash_icon(server5))
                        .spacing(8)
                    )
                    .push(Row::new()
                        .push("You:")
                        //TODO add gray background in light mode
                        .push(hash_icon(client1))
                        .push(hash_icon(client2))
                        .push(hash_icon(client3))
                        .push(hash_icon(client4))
                        .push(hash_icon(client5))
                        .spacing(8)
                    )
                    .push({
                        let mut row = Row::new();
                        row = row.push(Button::new("Delete Room").on_press(Message::SetRoomView(RoomView::ConfirmDeletion)));
                        if let Some(my_id) = self.last_world {
                            row = row.push(Button::new("Leave Room").on_press(Message::Kick(my_id)));
                        }
                        row.spacing(8)
                    })
                    .spacing(8)
                    .padding(8)
                    .into(),
                SessionState::Room { view: RoomView::Options, wrong_file_hash: None, autodelete_delta, allow_send_all, .. } => {
                    let mut col = Column::new()
                        .push(Button::new("Back").on_press(Message::SetRoomView(RoomView::Normal)))
                        .push("Automatically delete this room if no items are sent for:")
                        .push({
                            let mut values = vec![
                                DurationFormatter(Duration::from_secs(60 * 60 * 24)),
                                DurationFormatter(Duration::from_secs(60 * 60 * 24 * 7)),
                                DurationFormatter(Duration::from_secs(60 * 60 * 24 * 90)),
                            ];
                            if let Err(idx) = values.binary_search(&DurationFormatter(autodelete_delta)) {
                                values.insert(idx, DurationFormatter(autodelete_delta));
                            }
                            PickList::new(values, Some(DurationFormatter(autodelete_delta)), Message::SetAutoDeleteDelta)
                        });
                    if allow_send_all {
                        col = col.push(Row::new()
                            .push("Send all items from world:")
                            .push({
                                let mut input = TextInput::new("", &self.send_all_world).on_input(Message::SetSendAllWorld).on_paste(Message::SetSendAllWorld).width(Length::Fixed(32.0));
                                if self.send_all_world.parse::<NonZeroU8>().is_ok() {
                                    input = input.on_submit(Message::SendAll);
                                }
                                input
                            })
                            .spacing(8)
                        )
                        .push(Row::new()
                            .push({
                                let mut input = TextInput::new("Spoiler Log", &self.send_all_path).on_input(Message::SetSendAllPath).on_paste(Message::SetSendAllPath);
                                if self.send_all_world.parse::<NonZeroU8>().is_ok() {
                                    input = input.on_submit(Message::SendAll);
                                }
                                input
                            })
                            .push(Button::new("Browse…").on_press(Message::SendAllBrowse))
                            .push({
                                let mut btn = Button::new("Send");
                                if self.send_all_world.parse::<NonZeroU8>().is_ok() {
                                    btn = btn.on_press(Message::SendAll);
                                }
                                btn
                            })
                            .spacing(8)
                        );
                    }
                    col.spacing(8).padding(8).into()
                }
                SessionState::Room { view: RoomView::Normal, wrong_file_hash: None, ref players, num_unassigned_clients, .. } => {
                    let (players, other) = format_room_state(players, num_unassigned_clients, self.last_world);
                    let mut col = Column::new()
                        .push(Row::new()
                            .push(Button::new("Delete Room").on_press(Message::SetRoomView(RoomView::ConfirmDeletion)))
                            .push(Button::new("Options").on_press(Message::SetRoomView(RoomView::Options)))
                            .spacing(8)
                        )
                        .push(Scrollable::new(Row::new()
                            .push(Column::with_children(players.into_iter().map(|(player_id, player)| Row::new()
                                .push(Text::new(player))
                                .push(Button::new(if self.last_world.map_or(false, |my_id| my_id == player_id) { "Leave" } else { "Kick" }).on_press(Message::Kick(player_id)))
                                .into()
                            ).collect()))
                            .push(Space::with_width(Length::Shrink)) // to avoid overlap with the scrollbar
                            .spacing(16)
                        ));
                    if !other.is_empty() {
                        col = col.push(Text::new(other));
                    }
                    col
                        .spacing(8)
                        .padding(8)
                        .into()
                }
                SessionState::Closed => Column::new()
                    .push("You have been disconnected.")
                    .push(Button::new("Reconnect").on_press(Message::ReconnectToLobby))
                    .spacing(8)
                    .padding(8)
                    .into(),
            }
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = Vec::with_capacity(3);
        if self.updates_checked {
            subscriptions.push(match self.frontend {
                FrontendFlags::BizHawk { port, .. } => Subscription::from_recipe(subscriptions::Connection { port, frontend: self.frontend.clone(), log: self.log, connection_id: self.frontend_connection_id }),
                FrontendFlags::Pj64V4 => Subscription::from_recipe(subscriptions::Connection { port: frontend::PORT, frontend: self.frontend.clone(), log: self.log, connection_id: self.frontend_connection_id }),
                FrontendFlags::Pj64V3 => Subscription::from_recipe(subscriptions::Listener { frontend: self.frontend.clone(), log: self.log, connection_id: self.frontend_connection_id }),
            });
            if !matches!(self.server_connection, SessionState::Error { .. } | SessionState::Closed) {
                subscriptions.push(Subscription::from_recipe(subscriptions::Client { log: self.log, websocket_url: self.websocket_url.clone() }));
            }
            if let SessionState::Lobby { view: LobbyView::Login(provider), .. } = self.server_connection {
                subscriptions.push(Subscription::from_recipe(login::Subscription(provider)));
            }
        }
        Subscription::batch(subscriptions)
    }
}

fn error_view<'a>(context: impl Into<Cow<'a, str>>, e: &impl ToString, debug_info_copied: bool) -> Element<'a, Message> {
    Scrollable::new(Row::new()
        .push(Column::new()
            .push(Text::new("Error").size(24))
            .push(Text::new(context))
            .push(Text::new(e.to_string()))
            .push(Row::new()
                .push(Button::new("Copy debug info").on_press(Message::CopyDebugInfo))
                .push(if debug_info_copied { "Copied!" } else { "for pasting into Discord" })
                .spacing(8)
            )
            .push(Text::new("Support").size(24))
            .push("• Ask in #setup-support on the OoT Randomizer Discord. Feel free to ping @fenhl.")
            .push(Row::new()
                .push(Button::new("invite link").on_press(Message::DiscordInvite))
                .push(Button::new("direct channel link").on_press(Message::DiscordChannel))
                .spacing(8)
            )
            .push("• Ask in #general on the OoTR MW Tournament Discord.")
            .push(Row::new()
                .push("• Or ")
                .push(Button::new("open an issue").on_press(Message::NewIssue))
                .spacing(8)
            )
            .spacing(8)
            .padding(8)
        )
        .push(Space::with_width(Length::Shrink)) // to avoid overlap with the scrollbar
        .spacing(16)
    ).into()
}

#[derive(Debug, thiserror::Error)]
enum MainError {
    #[error(transparent)] Iced(#[from] iced::Error),
    #[error(transparent)] Icon(#[from] iced::window::icon::Error),
    #[error(transparent)] Io(#[from] io::Error),
    #[error(transparent)] Wheel(#[from] wheel::Error),
}

#[derive(clap::Subcommand)]
#[clap(rename_all = "lower")]
enum FrontendArgs {
    BizHawk {
        path: PathBuf,
        pid: Pid,
        local_bizhawk_version: Version,
        port: u16,
    },
    Pj64V4,
}

#[derive(clap::Parser)]
#[clap(version)]
struct CliArgs {
    #[clap(subcommand)]
    frontend: Option<FrontendArgs>,
}

#[wheel::main(debug)]
fn main(CliArgs { frontend }: CliArgs) -> Result<(), MainError> {
    Ok(State::run(Settings {
        window: window::Settings {
            size: (256, 256),
            icon: Some(icon::from_file_data(include_bytes!("../../../assets/icon.ico"), Some(ImageFormat::Ico))?),
            ..window::Settings::default()
        },
        ..Settings::with_flags(match frontend {
            None => FrontendFlags::Pj64V3,
            Some(FrontendArgs::BizHawk { path, pid, local_bizhawk_version, port }) => FrontendFlags::BizHawk { path, pid, local_bizhawk_version, port },
            Some(FrontendArgs::Pj64V4) => FrontendFlags::Pj64V4,
        })
    })?)
}
