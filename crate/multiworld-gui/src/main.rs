#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use {
    std::{
        borrow::Cow,
        collections::BTreeMap,
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
    dark_light::Mode::{
        Dark,
        Light,
    },
    enum_iterator::all,
    futures::{
        future,
        sink::SinkExt as _,
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
    if_chain::if_chain,
    ::image::ImageFormat,
    itertools::Itertools as _,
    log_lock::{
        Mutex,
        lock,
    },
    oauth2::{
        RefreshToken,
        TokenResponse as _,
        reqwest::async_http_client,
    },
    once_cell::sync::Lazy,
    ootr_utils::spoiler::HashIcon,
    open::that as open,
    rand::prelude::*,
    rfd::AsyncFileDialog,
    semver::Version,
    serenity::utils::MessageBuilder,
    sysinfo::Pid,
    tokio::{
        io::{
            self,
            AsyncWriteExt as _,
        },
        net::tcp::{
            OwnedReadHalf,
            OwnedWriteHalf,
        },
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
        config::Config,
        format_room_state,
        frontend::{
            self,
            Kind as Frontend,
        },
        github::Repo,
        ws::{
            ServerError,
            latest::{
                ClientMessage,
                ServerMessage,
            },
        },
    },
    crate::{
        persistent_state::PersistentState,
        subscriptions::WsSink,
    },
};
#[cfg(unix)] use {
    std::os::unix::fs::PermissionsExt as _,
    xdg::BaseDirectories,
};
#[cfg(windows)] use directories::ProjectDirs;
#[cfg(target_os = "linux")] use gio::traits::SettingsExt as _;

mod everdrive;
mod login;
mod persistent_state;
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
            writeln!(&*lock!(LOG), "{} {}: {msg:?}", Utc::now().format("%Y-%m-%d %H:%M:%S"), self.context).map_err(|e| async_proto::ReadError {
                context: async_proto::ErrorContext::Custom(format!("multiworld-gui::LoggingReader::read")),
                kind: e.into(),
            })?;
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
            writeln!(&*lock!(LOG), "{} {}: {msg:?}", Utc::now().format("%Y-%m-%d %H:%M:%S"), self.context).map_err(|e| async_proto::ReadError {
                context: async_proto::ErrorContext::Custom(format!("multiworld-gui::LoggingStream::read_owned")),
                kind: e.into(),
            })?;
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
            writeln!(&*lock!(LOG), "{} {}: {msg:?}", Utc::now().format("%Y-%m-%d %H:%M:%S"), self.context).map_err(|e| async_proto::WriteError {
                context: async_proto::ErrorContext::Custom(format!("multiworld-gui::LoggingWriter::write")),
                kind: e.into(),
            })?;
        }
        msg.write(&mut *lock!(self.inner)).await
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
            writeln!(&*lock!(LOG), "{} {}: {msg:?}", Utc::now().format("%Y-%m-%d %H:%M:%S"), self.context).map_err(|e| async_proto::WriteError {
                context: async_proto::ErrorContext::Custom(format!("multiworld-gui::LoggingSink::write")),
                kind: e.into(),
            })?;
        }
        msg.write_ws(&mut *lock!(self.inner)).await
    }
}

fn hash_icon(icon: HashIcon) -> Element<'static, Message> {
    match icon {
        HashIcon::Beans => Svg::new(svg::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/beans.svg")[..])).width(50).height(50).into(),
        HashIcon::BigMagic => Svg::new(svg::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/big-magic.svg")[..])).width(50).height(50).into(),
        HashIcon::Bombchu => Image::new(image::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/bombchu.png")[..])).width(50).height(50).into(),
        HashIcon::Boomerang => Svg::new(svg::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/boomerang.svg")[..])).width(50).height(50).into(),
        HashIcon::BossKey => Image::new(image::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/boss-key.png")[..])).width(50).height(50).into(),
        HashIcon::BottledFish => Image::new(image::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/bottled-fish.png")[..])).width(50).height(50).into(),
        HashIcon::BottledMilk => Image::new(image::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/bottled-milk.png")[..])).width(50).height(50).into(),
        HashIcon::Bow => Svg::new(svg::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/bow.svg")[..])).width(50).height(50).into(),
        HashIcon::Compass => Image::new(image::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/compass.png")[..])).width(50).height(50).into(),
        HashIcon::Cucco => Image::new(image::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/cucco.png")[..])).width(50).height(50).into(),
        HashIcon::DekuNut => Image::new(image::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/deku-nut.png")[..])).width(50).height(50).into(),
        HashIcon::DekuStick => Image::new(image::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/deku-stick.png")[..])).width(50).height(50).into(),
        HashIcon::FairyOcarina => Svg::new(svg::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/fairy-ocarina.svg")[..])).width(50).height(50).into(),
        HashIcon::Frog => Image::new(image::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/frog.png")[..])).width(50).height(50).into(),
        HashIcon::GoldScale => Svg::new(svg::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/gold-scale.svg")[..])).width(50).height(50).into(),
        HashIcon::HeartContainer => Image::new(image::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/heart-container.png")[..])).width(50).height(50).into(),
        HashIcon::HoverBoots => Image::new(image::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/hover-boots.png")[..])).width(50).height(50).into(),
        HashIcon::KokiriTunic => Image::new(image::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/kokiri-tunic.png")[..])).width(50).height(50).into(),
        HashIcon::LensOfTruth => Svg::new(svg::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/lens-of-truth.svg")[..])).width(50).height(50).into(),
        HashIcon::Longshot => Svg::new(svg::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/longshot.svg")[..])).width(50).height(50).into(),
        HashIcon::Map => Image::new(image::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/map.png")[..])).width(50).height(50).into(),
        HashIcon::MaskOfTruth => Svg::new(svg::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/mask-of-truth.svg")[..])).width(50).height(50).into(),
        HashIcon::MasterSword => Svg::new(svg::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/master-sword.svg")[..])).width(50).height(50).into(),
        HashIcon::MegatonHammer => Svg::new(svg::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/megaton-hammer.svg")[..])).width(50).height(50).into(),
        HashIcon::MirrorShield => Svg::new(svg::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/mirror-shield.svg")[..])).width(50).height(50).into(),
        HashIcon::Mushroom => Image::new(image::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/mushroom.png")[..])).width(50).height(50).into(),
        HashIcon::Saw => Image::new(image::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/saw.png")[..])).width(50).height(50).into(),
        HashIcon::SilverGauntlets => Svg::new(svg::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/silver-gauntlets.svg")[..])).width(50).height(50).into(),
        HashIcon::SkullToken => Svg::new(svg::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/skull-token.svg")[..])).width(50).height(50).into(),
        HashIcon::Slingshot => Svg::new(svg::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/slingshot.svg")[..])).width(50).height(50).into(),
        HashIcon::SoldOut => Image::new(image::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/sold-out.png")[..])).width(50).height(50).into(),
        HashIcon::StoneOfAgony => Image::new(image::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/stone-of-agony.png")[..])).width(50).height(50).into(),
    }
}

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)] Client(#[from] multiworld::ClientError),
    #[error(transparent)] Config(#[from] multiworld::config::Error),
    #[error(transparent)] Elapsed(#[from] tokio::time::error::Elapsed),
    #[error(transparent)] Io(#[from] io::Error),
    #[error(transparent)] Json(#[from] serde_json::Error),
    #[error(transparent)] PersistentState(#[from] persistent_state::Error),
    #[error(transparent)] Read(#[from] async_proto::ReadError),
    #[error(transparent)] RequestToken(#[from] oauth2::basic::BasicRequestTokenError<oauth2::reqwest::HttpClientError>),
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
        frontend: Frontend,
        version: u8,
    },
}

impl IsNetworkError for Error {
    fn is_network_error(&self) -> bool {
        match self {
            Self::Elapsed(_) => true,
            Self::Config(_) | Self::Json(_) | Self::PersistentState(_) | Self::Semver(_) | Self::Url(_) | Self::CopyDebugInfo | Self::VersionMismatch { .. } => false,
            Self::Client(e) => e.is_network_error(),
            Self::Io(e) => e.is_network_error(),
            Self::Read(e) => e.is_network_error(),
            Self::RequestToken(e) => match e {
                oauth2::basic::BasicRequestTokenError::ServerResponse(_) => false,
                oauth2::basic::BasicRequestTokenError::Request(e) => match e {
                    oauth2::reqwest::Error::Reqwest(e) => e.is_network_error(),
                    oauth2::reqwest::Error::Http(_) => false, // this is https://docs.rs/http/0.2.9/http/struct.Error.html which does not appear to be constructed from network errors
                    oauth2::reqwest::Error::Io(e) => e.is_network_error(),
                    oauth2::reqwest::Error::Other(_) => false,
                },
                oauth2::basic::BasicRequestTokenError::Parse(_, _) => false,
                oauth2::basic::BasicRequestTokenError::Other(_) => false,
            },
            Self::Reqwest(e) => e.is_network_error(),
            Self::WebSocket(e) => e.is_network_error(),
            Self::Wheel(e) => e.is_network_error(),
            Self::Write(e) => e.is_network_error(),
            #[cfg(unix)] Self::Xdg(_) => false,
            #[cfg(windows)] Self::MissingHomeDir => false,
        }
    }
}

#[derive(Debug, Clone)]
enum Message {
    CheckForUpdates,
    CommandError(Arc<Error>),
    ConfirmRoomDeletion,
    CopyDebugInfo,
    CreateMidosHouseAccount(login::Provider),
    DiscordChannel,
    DiscordInvite,
    DismissWrongPassword,
    Event(iced::Event),
    Exit,
    FrontendConnected(Arc<Mutex<OwnedWriteHalf>>),
    FrontendSubscriptionError(Arc<Error>),
    JoinRoom,
    Kick(NonZeroU8),
    Leave,
    LoginError(Arc<login::Error>),
    LoginTokens {
        provider: login::Provider,
        bearer_token: String,
        refresh_token: Option<String>,
    },
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
    SetFrontend(Frontend),
    SetLobbyView(LobbyView),
    SetNewRoomName(String),
    SetPassword(String),
    SetRoomView(RoomView),
    SetSendAllPath(String),
    SetSendAllWorld(String),
    UpToDate,
}

fn cmd(future: impl Future<Output = Result<Message, Error>> + Send + 'static) -> Command<Message> {
    Command::single(iced_runtime::command::Action::Future(Box::pin(async move {
        match future.await {
            Ok(msg) => msg,
            Err(e) => Message::CommandError(Arc::new(e.into())),
        }
    })))
}

struct State {
    persistent_state: PersistentState,
    frontend: FrontendState,
    debug_info_copied: bool,
    icon_error: Option<Arc<iced::window::icon::Error>>,
    config_error: Option<Arc<multiworld::config::Error>>,
    persistent_state_error: Option<Arc<persistent_state::Error>>,
    command_error: Option<Arc<Error>>,
    login_error: Option<Arc<login::Error>>,
    frontend_subscription_error: Option<Arc<Error>>,
    frontend_connection_id: u8,
    frontend_writer: Option<LoggingWriter>,
    log: bool,
    login_tokens: BTreeMap<login::Provider, String>,
    refresh_tokens: BTreeMap<login::Provider, String>,
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
    updates_checked: bool,
    send_all_path: String,
    send_all_world: String,
}

impl State {
    fn error_to_markdown(&self) -> Option<String> {
        Some(if let Some(ref e) = self.icon_error {
            MessageBuilder::default()
                .push_line(concat!("error in Mido's House Multiworld version ", env!("CARGO_PKG_VERSION"), " while trying to load icon:"))
                .push_line_safe(e.to_string())
                .push_codeblock_safe(format!("{e:?}"), Some("rust"))
                .build()
        } else if let Some(ref e) = self.config_error {
            MessageBuilder::default()
                .push_line(concat!("error in Mido's House Multiworld version ", env!("CARGO_PKG_VERSION"), " while trying to load config:"))
                .push_line_safe(e.to_string())
                .push_codeblock_safe(format!("{e:?}"), Some("rust"))
                .build()
        } else if let Some(ref e) = self.persistent_state_error {
            MessageBuilder::default()
                .push_line(concat!("error in Mido's House Multiworld version ", env!("CARGO_PKG_VERSION"), " while trying to load persistent state:"))
                .push_line_safe(e.to_string())
                .push_codeblock_safe(format!("{e:?}"), Some("rust"))
                .build()
        } else if let Some(ref e) = self.command_error {
            MessageBuilder::default()
                .push_line(concat!("error in Mido's House Multiworld version ", env!("CARGO_PKG_VERSION"), ":"))
                .push_line_safe(e.to_string())
                .push_codeblock_safe(format!("{e:?}"), Some("rust"))
                .build()
        } else if let Some(ref e) = self.login_error {
            let with_provider = if let SessionState::Lobby { view: LobbyView::Login { provider, .. }, .. } = self.server_connection {
                format!(" with {provider}")
            } else {
                String::default()
            };
            MessageBuilder::default()
                .push_line(format!("error in Mido's House Multiworld version {} while trying to sign in{with_provider}:", env!("CARGO_PKG_VERSION")))
                .push_line_safe(e.to_string())
                .push_codeblock_safe(format!("{e:?}"), Some("rust"))
                .build()
        } else if let Some(ref e) = self.frontend_subscription_error {
            MessageBuilder::default()
                .push_line(format!("error in Mido's House Multiworld version {} during communication with {}:", env!("CARGO_PKG_VERSION"), self.frontend.display_with_version()))
                .push_line_safe(e.to_string())
                .push_codeblock_safe(format!("{e:?}"), Some("rust"))
                .build()
        } else if let SessionState::Error { ref e, .. } = self.server_connection {
            MessageBuilder::default()
                .push_line(if_chain! {
                    if let SessionStateError::Connection(e) = e;
                    if e.is_network_error();
                    then {
                        concat!("network error in Mido's House Multiworld version ", env!("CARGO_PKG_VERSION"), ":")
                    } else {
                        concat!("error in Mido's House Multiworld version ", env!("CARGO_PKG_VERSION"), " during communication with the server:")
                    }
                })
                .push_line_safe(e.to_string())
                .push_codeblock_safe(format!("{e:?}"), Some("rust"))
                .build()
        } else {
            return None
        })
    }
}

#[derive(Debug, Clone)]
struct BizHawkState {
    path: PathBuf,
    pid: Pid,
    version: Version,
    port: u16,
}

#[derive(Debug, Clone)]
struct FrontendState {
    kind: Frontend,
    bizhawk: Option<BizHawkState>,
}

impl FrontendState {
    fn display_with_version(&self) -> Cow<'static, str> {
        match self.kind {
            Frontend::Dummy => "(no frontend)".into(),
            Frontend::EverDrive => "EverDrive".into(),
            Frontend::BizHawk => if let Some(BizHawkState { ref version, .. }) = self.bizhawk {
                format!("BizHawk {version}").into()
            } else {
                "BizHawk".into()
            },
            Frontend::Pj64V3 => "Project64 3.x".into(),
            Frontend::Pj64V4 => "Project64 4.x".into(),
        }
    }

    fn is_locked(&self) -> bool {
        match self.kind {
            Frontend::Dummy | Frontend::EverDrive | Frontend::Pj64V3 => false,
            Frontend::Pj64V4 => false, //TODO pass port from PJ64, consider locked if present
            Frontend::BizHawk => self.bizhawk.is_some(),
        }
    }
}

impl Application for State {
    type Executor = iced::executor::Default;
    type Message = Message;
    type Theme = Theme;
    type Flags = (Option<iced::window::icon::Error>, Result<Config, multiworld::config::Error>, Result<PersistentState, persistent_state::Error>, Option<FrontendArgs>);

    fn new((icon_error, config, persistent_state, frontend): Self::Flags) -> (Self, Command<Message>) {
        let (config, config_error) = match config {
            Ok(config) => (config, None),
            Err(e) => (Config::default(), Some(Arc::new(e))),
        };
        let (persistent_state, persistent_state_error) = match persistent_state {
            Ok(persistent_state) => (persistent_state, None),
            Err(e) => (PersistentState::default(), Some(Arc::new(e))),
        };
        let frontend = FrontendState {
            kind: match frontend {
                None => config.default_frontend.unwrap_or(Frontend::Pj64V3),
                Some(FrontendArgs::Dummy) => Frontend::Dummy,
                Some(FrontendArgs::EverDrive) => Frontend::EverDrive,
                Some(FrontendArgs::BizHawk { .. }) => Frontend::BizHawk,
                Some(FrontendArgs::Pj64V3) => Frontend::Pj64V3,
                Some(FrontendArgs::Pj64V4) => Frontend::Pj64V4,
            },
            bizhawk: if let Some(FrontendArgs::BizHawk { path, pid, version, port }) = frontend {
                Some(BizHawkState { path, pid, version, port })
            } else {
                None
            },
        };
        (Self {
            debug_info_copied: false,
            icon_error: icon_error.map(Arc::new),
            command_error: None,
            login_error: None,
            frontend_subscription_error: None,
            frontend_connection_id: 0,
            frontend_writer: None,
            websocket_url: config.websocket_url().expect("failed to parse WebSocket URL"),
            log: config.log,
            login_tokens: config.login_tokens,
            refresh_tokens: config.refresh_tokens,
            last_login_url: None,
            server_connection: SessionState::Init,
            server_writer: None,
            retry: Instant::now(),
            wait_time: Duration::from_secs(1),
            last_world: None,
            last_name: Filename::default(),
            last_hash: None,
            last_save: None,
            updates_checked: false,
            send_all_path: String::default(),
            send_all_world: String::default(),
            frontend, config_error, persistent_state_error, persistent_state,
        }, cmd(future::ok(Message::CheckForUpdates)))
    }

    fn theme(&self) -> Theme {
        //TODO automatically update on system theme change
        #[cfg(target_os = "linux")] {
            let settings = gio::Settings::new("org.gnome.desktop.interface");
            if settings.settings_schema().map_or(false, |schema| schema.has_key("color-scheme")) {
                match settings.string("color-scheme").as_str() {
                    "prefer-light" => return Theme::Light,
                    "prefer-dark" => return Theme::Dark,
                    _ => {}
                }
            }
        }
        match dark_light::detect() {
            Dark => Theme::Dark,
            Light | dark_light::Mode::Default => Theme::Light,
        }
    }

    fn title(&self) -> String {
        if self.frontend.is_locked() {
            format!("Mido's House Multiworld for {}", self.frontend.kind)
        } else {
            format!("Mido's House Multiworld")
        }
    }

    fn update(&mut self, msg: Message) -> Command<Message> {
        match msg {
            Message::SetLobbyView(new_view) => if let SessionState::Lobby { ref mut view, .. } = self.server_connection {
                *view = new_view;
            },
            Message::SetRoomView(new_view) => if let SessionState::Room { ref mut view, .. } = self.server_connection {
                *view = new_view;
            },
            Message::CheckForUpdates => {
                let frontend = self.frontend.clone();
                return cmd(async move {
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
                            match frontend.kind {
                                Frontend::Dummy => return Ok(Message::UpToDate),
                                Frontend::EverDrive => {
                                    cmd.arg("everdrive");
                                    cmd.arg(env::current_exe()?);
                                    cmd.arg(process::id().to_string());
                                }
                                Frontend::BizHawk => if let Some(BizHawkState { path, pid, version, port: _ }) = frontend.bizhawk {
                                    cmd.arg("bizhawk");
                                    cmd.arg(process::id().to_string());
                                    cmd.arg(path);
                                    cmd.arg(pid.to_string());
                                    cmd.arg(version.to_string());
                                } else {
                                    return Ok(Message::UpToDate)
                                },
                                Frontend::Pj64V3 | Frontend::Pj64V4 => {
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
                })
            }
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
            Message::CreateMidosHouseAccount(provider) => if let Err(e) = open(match provider {
                login::Provider::Discord => "https://midos.house/login/discord",
                login::Provider::RaceTime => "https://midos.house/login/racetime",
            }) {
                return cmd(future::err(e.into()))
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
            Message::Event(iced::Event::Window(iced::window::Event::CloseRequested)) => if self.command_error.is_some() || self.login_error.is_some() || self.frontend_subscription_error.is_some() {
                return window::close()
            } else {
                let frontend_writer = self.frontend_writer.take();
                let server_writer = self.server_writer.take();
                return cmd(async move {
                    if let Some(frontend_writer) = frontend_writer {
                        lock!(frontend_writer.inner).shutdown().await?;
                    }
                    if let Some(server_writer) = server_writer {
                        let mut server_writer = lock!(server_writer.inner);
                        server_writer.send(tungstenite::Message::Close(Some(tungstenite::protocol::CloseFrame {
                            code: tungstenite::protocol::frame::coding::CloseCode::Away,
                            reason: "multiworld app exiting".into(),
                        }))).await?;
                        server_writer.close().await?;
                    }
                    Ok(Message::Exit)
                })
            },
            Message::Event(_) => {}
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
                if let Error::Read(async_proto::ReadError { kind: async_proto::ReadErrorKind::Io(ref e), .. }) = *e {
                    match (self.frontend.kind, e.kind()) {
                        (Frontend::BizHawk | Frontend::Pj64V4, io::ErrorKind::ConnectionReset | io::ErrorKind::UnexpectedEof) => return window::close(), // frontend closed
                        (Frontend::Pj64V3, io::ErrorKind::ConnectionReset) => {
                            self.frontend_writer = None;
                            return Command::none()
                        }
                        (_, _) => {}
                    }
                }
                self.frontend_subscription_error.get_or_insert(e);
            }
            Message::JoinRoom => if let SessionState::Lobby { create_new_room, ref existing_room_selection, ref new_room_name, ref password, .. } = self.server_connection {
                if !password.is_empty() || existing_room_selection.as_ref().is_some_and(|room| !room.password_required) {
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
            Message::Leave => if let Some(writer) = self.server_writer.clone() {
                return cmd(async move {
                    writer.write(ClientMessage::LeaveRoom).await?;
                    Ok(Message::Nop)
                })
            },
            Message::LoginError(e) => { self.login_error.get_or_insert(e); }
            Message::LoginTokens { provider, bearer_token, refresh_token } => {
                self.login_tokens.insert(provider, bearer_token.clone());
                if let Some(ref refresh_token) = refresh_token {
                    self.refresh_tokens.insert(provider, refresh_token.clone());
                }
                if let Some(writer) = self.server_writer.clone() {
                    let change_view = matches!(self.server_connection, SessionState::Lobby { view: LobbyView::Login { .. }, .. });
                    return cmd(async move {
                        let mut config = Config::load().await?;
                        config.login_tokens.insert(provider, bearer_token.clone());
                        if let Some(refresh_token) = refresh_token {
                            config.refresh_tokens.insert(provider, refresh_token.clone());
                        }
                        config.save().await?;
                        writer.write(match provider {
                            login::Provider::RaceTime => ClientMessage::LoginRaceTime { bearer_token },
                            login::Provider::Discord => ClientMessage::LoginDiscord { bearer_token },
                        }).await?;
                        Ok(if change_view {
                            Message::SetLobbyView(LobbyView::Settings)
                        } else {
                            Message::Nop
                        })
                    })
                }
            }
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
                        let persistent_state = self.persistent_state.clone();
                        return cmd(async move {
                            persistent_state.edit(move |state| state.pending_items_after_save.push((key, kind, target_world))).await?;
                            Ok(Message::Nop)
                        })
                    }
                }
                frontend::ClientMessage::SaveData(save) => match oottracker::Save::from_save_data(&save) {
                    Ok(save) => {
                        self.last_save = Some(save.clone());
                        let persistent_state = self.persistent_state.clone();
                        let writer = if let SessionState::Room { .. } = self.server_connection {
                            self.server_writer.clone()
                        } else {
                            None
                        };
                        return cmd(async move {
                            persistent_state.edit(|state| state.pending_items_before_save.extend(state.pending_items_after_save.drain(..))).await?;
                            if let Some(writer) = writer {
                                writer.write(ClientMessage::SaveData(save)).await?;
                            }
                            Ok(Message::Nop)
                        })
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
                    ServerMessage::StructuredError(ServerError::NoMidosHouseAccountDiscord) => {
                        self.login_tokens.remove(&login::Provider::Discord);
                        self.refresh_tokens.remove(&login::Provider::Discord);
                        return cmd(async {
                            let mut config = Config::load().await?;
                            config.login_tokens.remove(&login::Provider::Discord);
                            config.refresh_tokens.remove(&login::Provider::Discord);
                            config.save().await?;
                            Ok(Message::Nop)
                        })
                    }
                    ServerMessage::StructuredError(ServerError::NoMidosHouseAccountRaceTime) => {
                        self.login_tokens.remove(&login::Provider::RaceTime);
                        self.refresh_tokens.remove(&login::Provider::RaceTime);
                        return cmd(async {
                            let mut config = Config::load().await?;
                            config.login_tokens.remove(&login::Provider::RaceTime);
                            config.refresh_tokens.remove(&login::Provider::RaceTime);
                            config.save().await?;
                            Ok(Message::Nop)
                        })
                    }
                    ServerMessage::StructuredError(ServerError::SessionExpiredDiscord) => {
                        self.login_tokens.remove(&login::Provider::Discord);
                        if let Some(refresh_token) = self.refresh_tokens.remove(&login::Provider::Discord) {
                            return cmd(async move {
                                let tokens = login::oauth_client(login::Provider::Discord)?
                                    .exchange_refresh_token(&RefreshToken::new(refresh_token))
                                    .request_async(async_http_client).await?;
                                Ok(Message::LoginTokens {
                                    provider: login::Provider::Discord,
                                    bearer_token: tokens.access_token().secret().clone(),
                                    refresh_token: tokens.refresh_token().map(|refresh_token| refresh_token.secret().clone()),
                                })
                            })
                        } else {
                            if let SessionState::Lobby { ref mut view, .. } = self.server_connection {
                                *view = LobbyView::SessionExpired(login::Provider::Discord);
                            }
                        }
                    }
                    ServerMessage::StructuredError(ServerError::SessionExpiredRaceTime) => {
                        self.login_tokens.remove(&login::Provider::RaceTime);
                        if let Some(refresh_token) = self.refresh_tokens.remove(&login::Provider::RaceTime) {
                            return cmd(async move {
                                let tokens = login::oauth_client(login::Provider::RaceTime)?
                                    .exchange_refresh_token(&RefreshToken::new(refresh_token))
                                    .request_async(async_http_client).await?;
                                Ok(Message::LoginTokens {
                                    provider: login::Provider::RaceTime,
                                    bearer_token: tokens.access_token().secret().clone(),
                                    refresh_token: tokens.refresh_token().map(|refresh_token| refresh_token.secret().clone()),
                                })
                            })
                        } else {
                            if let SessionState::Lobby { ref mut view, .. } = self.server_connection {
                                *view = LobbyView::SessionExpired(login::Provider::RaceTime);
                            }
                        }
                    }
                    ServerMessage::EnterLobby { .. } => {
                        let login_token = self.login_tokens.iter()
                            .next()
                            .filter(|_| matches!(self.server_connection, SessionState::Lobby { login_state: None, .. }))
                            .map(|(&provider, bearer_token)| (provider, bearer_token.clone()));
                        let server_writer = self.server_writer.clone();
                        let frontend_writer = self.frontend_writer.clone();
                        return cmd(async move {
                            if let Some(server_writer) = server_writer {
                                if let Some((provider, bearer_token)) = login_token {
                                    server_writer.write(match provider {
                                        login::Provider::RaceTime => ClientMessage::LoginRaceTime { bearer_token },
                                        login::Provider::Discord => ClientMessage::LoginDiscord { bearer_token },
                                    }).await?;
                                }
                            }
                            if let Some(frontend_writer) = frontend_writer {
                                frontend_writer.write(frontend::ServerMessage::ItemQueue(Vec::default())).await?;
                            }
                            Ok(if room_still_exists { Message::JoinRoom } else { Message::Nop })
                        })
                    }
                    ServerMessage::EnterRoom { players, .. } => {
                        let persistent_state = self.persistent_state.clone();
                        let server_writer = self.server_writer.clone().expect("join room button only appears when connected to server");
                        let frontend_writer = self.frontend_writer.clone().expect("join room button only appears when connected to frontend");
                        let player_id = self.last_world;
                        let player_name = self.last_name;
                        let file_hash = self.last_hash;
                        let save = self.last_save.clone();
                        return cmd(async move {
                            let (pending_items_before_save, pending_items_after_save) = persistent_state.edit(|state| (
                                mem::take(&mut state.pending_items_before_save),
                                mem::take(&mut state.pending_items_after_save),
                            )).await?;
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
                                server_writer.write(ClientMessage::SaveData(save)).await?;
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
                    ServerMessage::ItemQueue(queue) => if let SessionState::Room { wrong_file_hash: None, world_taken: None, .. } = self.server_connection {
                        if let Some(writer) = self.frontend_writer.clone() {
                            return cmd(async move {
                                writer.write(frontend::ServerMessage::ItemQueue(queue)).await?;
                                Ok(Message::Nop)
                            })
                        }
                    },
                    ServerMessage::GetItem(item) => if let SessionState::Room { wrong_file_hash: None, world_taken: None, .. } = self.server_connection {
                        if let Some(writer) = self.frontend_writer.clone() {
                            return cmd(async move {
                                writer.write(frontend::ServerMessage::GetItem(item)).await?;
                                Ok(Message::Nop)
                            })
                        }
                    },
                    ServerMessage::ProgressiveItems { world, state } => if let SessionState::Room { wrong_file_hash: None, world_taken: None, .. } = self.server_connection {
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
                        self.wait_time = self.wait_time.mul_f64(thread_rng().gen_range(1.0..=2.0)); // randomized exponential backoff
                    }
                    self.retry = Instant::now() + self.wait_time;
                    let retry = self.retry;
                    let reconnect_msg = if let SessionState::Room { room_id, ref room_password, .. } = self.server_connection {
                        Message::ReconnectToRoom(room_id, room_password.clone())
                    } else {
                        Message::ReconnectToLobby
                    };
                    self.server_connection = SessionState::Error {
                        maintenance: self.server_connection.maintenance(),
                        e: SessionStateError::Connection(e),
                        auto_retry: true,
                    };
                    return cmd(async move {
                        sleep_until(retry).await;
                        Ok(reconnect_msg)
                    })
                } else {
                    self.server_connection = SessionState::Error {
                        maintenance: self.server_connection.maintenance(),
                        e: SessionStateError::Connection(e.clone()),
                        auto_retry: false,
                    };
                    if let Error::WebSocket(tungstenite::Error::Http(ref resp)) = *e {
                        if resp.status() == tungstenite::http::StatusCode::GONE {
                            self.updates_checked = false;
                            return cmd(future::ok(Message::CheckForUpdates))
                        }
                    }
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
            Message::SetFrontend(new_frontend) => self.frontend.kind = new_frontend,
            Message::SetNewRoomName(name) => if let SessionState::Lobby { ref mut new_room_name, .. } = self.server_connection { *new_room_name = name },
            Message::SetPassword(new_password) => if let SessionState::Lobby { ref mut password, .. } = self.server_connection { *password = new_password },
            Message::SetSendAllPath(new_path) => self.send_all_path = new_path,
            Message::SetSendAllWorld(new_world) => self.send_all_world = new_world,
            Message::UpToDate => self.updates_checked = true,
        }
        Command::none()
    }

    fn view(&self) -> Element<'_, Message> {
        if let Some(ref e) = self.icon_error {
            error_view("An error occurred:", e, self.debug_info_copied)
        } else if let Some(ref e) = self.config_error {
            error_view("An error occurred:", e, self.debug_info_copied)
        } else if let Some(ref e) = self.persistent_state_error {
            error_view("An error occurred:", e, self.debug_info_copied)
        } else if let Some(ref e) = self.command_error {
            error_view("An error occurred:", e, self.debug_info_copied)
        } else if let Some(ref e) = self.login_error {
            error_view("An error occurred while trying to sign in:", e, self.debug_info_copied) //TODO button to reset error state
        } else if let Some(ref e) = self.frontend_subscription_error {
            if let Error::Io(ref e) = **e {
                if e.kind() == io::ErrorKind::AddrInUse {
                    Column::new()
                        .push(Text::new("Connection Busy").size(24))
                        .push(Text::new(format!("Could not connect to {} because the connection is already in use. Maybe you still have another instance of this app open?", self.frontend.kind)))
                        .push(Button::new("Retry").on_press(Message::ReconnectFrontend))
                        .spacing(8)
                        .padding(8)
                        .into()
                } else {
                    error_view(format!("An error occurred during communication with {}:", self.frontend.kind), e, self.debug_info_copied)
                }
            } else {
                error_view(format!("An error occurred during communication with {}:", self.frontend.kind), e, self.debug_info_copied)
            }
        } else if !self.updates_checked {
            let mut col = Column::new();
            if let SessionState::Error { auto_retry: false, e: SessionStateError::Connection(ref e), maintenance } = self.server_connection {
                if let Error::WebSocket(tungstenite::Error::Http(ref resp)) = **e {
                    if resp.status() == tungstenite::http::StatusCode::GONE {
                        if let Some((start, duration)) = maintenance {
                            col = col.push(Text::new(format!(
                                "Maintenance on the Mido's House server is scheduled for {} (time shown in your local timezone). Mido's House Multiworld is expected to go offline for approximately {}.",
                                start.with_timezone(&Local).format("%A, %B %e, %H:%M"),
                                DurationFormatter(duration),
                            )));
                        }
                        col = col.push("This version of the multiworld app is no longer supported by the server.");
                    }
                }
            }
            col
                .push("Checking for updates…")
                .spacing(8)
                .padding(8)
                .into()
        } else if self.frontend_writer.is_none() && self.frontend.kind != Frontend::Dummy {
            let mut col = Column::new();
            if !self.frontend.is_locked() {
                col = col.push(PickList::new(all::<Frontend>().filter(|&iter_frontend| self.frontend.kind == iter_frontend || iter_frontend.is_supported()).collect_vec(), Some(self.frontend.kind), Message::SetFrontend));
            }
            match self.frontend.kind {
                Frontend::Dummy => unreachable!(),
                Frontend::EverDrive => {
                    col = col
                        .push("Looking for EverDrives…")
                        .push("Make sure your console is turned on and connected, and your USB cable supports data.");
                }
                Frontend::BizHawk => if self.frontend.bizhawk.is_some() {
                    col = col
                        .push("Waiting for BizHawk…")
                        .push("Make sure your game is running and unpaused.");
                } else {
                    col = col
                        .push("BizHawk not connected")
                        .push("To use multiworld with BizHawk, start it from BizHawk's Tools → External Tools menu.");
                },
                Frontend::Pj64V3 => {
                    col = col
                        .push("Waiting for Project64…")
                        .push("1. In Project64's Debugger menu, select Scripts\n2. In the Scripts window, select ootrmw.js and click Run\n3. Wait until the Output area says “Connected to multiworld app”. (This should take less than 5 seconds.) You can then close the Scripts window.");
                }
                Frontend::Pj64V4 => {
                    col = col
                        .push("Waiting for Project64…")
                        .push("This should take less than 5 seconds.");
                }
            }
            col.spacing(8).padding(8).into()
        } else {
            match self.server_connection {
                SessionState::Error { auto_retry: false, ref e, maintenance: _ } => error_view("An error occurred during communication with the server:", e, self.debug_info_copied),
                SessionState::Error { auto_retry: true, ref e, maintenance } => {
                    let mut col = Column::new();
                    if let Some((start, duration)) = maintenance {
                        col = col.push(Text::new(format!(
                            "Maintenance on the Mido's House server is scheduled for {} (time shown in your local timezone). Mido's House Multiworld is expected to go offline for approximately {}.",
                            start.with_timezone(&Local).format("%A, %B %e, %H:%M"),
                            DurationFormatter(duration),
                        )));
                    }
                    col
                        .push("A network error occurred:")
                        .push(Text::new(e.to_string()))
                        .push(Text::new(if let Ok(retry) = chrono::Duration::from_std(self.retry.duration_since(Instant::now())) {
                            format!("Reconnecting at {}", (Local::now() + retry).format("%H:%M:%S"))
                        } else {
                            format!("Reconnecting…")
                        })) //TODO live countdown
                        .push("If this error persists, check your internet connection or contact @fenhl on Discord for support.")
                        .push(Row::new()
                            .push(Button::new("Copy debug info").on_press(Message::CopyDebugInfo))
                            .push(if self.debug_info_copied { "Copied!" } else { "for pasting into Discord" })
                            .spacing(8)
                        )
                        .spacing(8)
                        .padding(8)
                        .into()
                }
                SessionState::Init => Column::new()
                    .push("Connecting to server…")
                    .push("If this takes longer than 5 seconds, check your internet connection or contact @fenhl on Discord for support.")
                    .push(Space::with_height(Length::Fill))
                    .push(concat!("version ", env!("CARGO_PKG_VERSION")))
                    .spacing(8)
                    .padding(8)
                    .into(),
                SessionState::InitAutoRejoin { .. } => Column::new()
                    .push("Reconnecting to room…")
                    .push("If this takes longer than 5 seconds, check your internet connection or contact @fenhl on Discord for support.")
                    .push(Space::with_height(Length::Fill))
                    .push(concat!("version ", env!("CARGO_PKG_VERSION")))
                    .spacing(8)
                    .padding(8)
                    .into(),
                SessionState::Lobby { wrong_password: true, .. } => Column::new()
                    .push("wrong password")
                    .push(Space::with_height(Length::Fill))
                    .push(Button::new("OK").on_press(Message::DismissWrongPassword))
                    .spacing(8)
                    .padding(8)
                    .into(),
                SessionState::Lobby { view: LobbyView::SessionExpired(provider), wrong_password: false, .. } => Column::new()
                    .push(Text::new(format!("Your Mido's House user session has expired.")))
                    .push(Button::new("Sign back in").on_press(Message::SetLobbyView(LobbyView::Login { provider, no_midos_house_account: false })))
                    .push(Space::with_width(Length::Fill))
                    .push(Button::new("Cancel").on_press(Message::SetLobbyView(LobbyView::Normal)))
                    .spacing(8)
                    .padding(8)
                    .into(),
                SessionState::Lobby { view: LobbyView::Settings, wrong_password: false, login_state, .. } => {
                    let mut col = Column::new()
                        .push(Row::new()
                            .push(Button::new("Back").on_press(Message::SetLobbyView(LobbyView::Normal)))
                            .push(Space::with_width(Length::Fill))
                            .push(concat!("version ", env!("CARGO_PKG_VERSION")))
                        );
                    if login_state.is_some() {
                        col = col.push("You are signed in."); //TODO option to sign out
                    } else {
                        col = col
                            .push("To access official tournament rooms, sign into Mido's House:")
                            .push(Button::new("Sign in with racetime.gg").on_press(Message::SetLobbyView(LobbyView::Login { provider: login::Provider::RaceTime, no_midos_house_account: false })))
                            .push(Button::new("Sign in with Discord").on_press(Message::SetLobbyView(LobbyView::Login { provider: login::Provider::Discord, no_midos_house_account: false })));
                    }
                    col.spacing(8).padding(8).into()
                }
                SessionState::Lobby { view: LobbyView::Login { provider, no_midos_house_account: true }, wrong_password: false, .. } => Column::new()
                    .push(Text::new(format!("This {provider} account is not associated with a Mido's House account.")))
                    .push(Row::new()
                        .push(Button::new("Create a Mido's House account").on_press(Message::CreateMidosHouseAccount(provider)))
                        .push(",")
                        .spacing(8)
                    )
                    .push(Row::new()
                        .push("then")
                        .push(Button::new("try again").on_press(Message::SetLobbyView(LobbyView::Login { provider, no_midos_house_account: false })))
                        .push(".")
                        .spacing(8)
                    )
                    .push(Space::with_height(Length::Fill))
                    .push(Button::new("Cancel").on_press(Message::SetLobbyView(LobbyView::Settings)))
                    .spacing(8)
                    .padding(8)
                    .into(),
                SessionState::Lobby { view: LobbyView::Login { provider, no_midos_house_account: false }, wrong_password: false, .. } => Column::new()
                    .push(Text::new(format!("Signing in with {provider}…")))
                    .push("Please continue in your web browser.")
                    .push({
                        let mut btn = Button::new("Reopen Web Page");
                        if let Some(ref last_login_url) = self.last_login_url {
                            btn = btn.on_press(Message::OpenLoginPage(last_login_url.clone()));
                        }
                        btn
                    })
                    .push(Space::with_height(Length::Fill))
                    .push(Button::new("Cancel").on_press(Message::SetLobbyView(LobbyView::Settings)))
                    .spacing(8)
                    .padding(8)
                    .into(),
                SessionState::Lobby { view: LobbyView::Normal, wrong_password: false, ref rooms, create_new_room, ref existing_room_selection, ref new_room_name, ref password, maintenance, .. } => {
                    let mut col = Column::new();
                    if let Some((start, duration)) = maintenance {
                        col = col.push(Text::new(format!(
                            "Maintenance on the Mido's House server is scheduled for {} (time shown in your local timezone). Mido's House Multiworld is expected to go offline for approximately {}.",
                            start.with_timezone(&Local).format("%A, %B %e, %H:%M"),
                            DurationFormatter(duration),
                        )));
                    }
                    col = col
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
                                let enabled = if create_new_room {
                                    !new_room_name.is_empty() && !password.is_empty()
                                } else {
                                    existing_room_selection.as_ref().is_some_and(|existing_room_selection| !existing_room_selection.password_required || !password.is_empty())
                                };
                                if enabled { btn = btn.on_press(Message::JoinRoom) }
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
                    .push(Button::new("Delete").on_press(Message::ConfirmRoomDeletion))
                    .push(Space::with_height(Length::Fill))
                    .push(Button::new("Back").on_press(Message::SetRoomView(RoomView::Normal)))
                    .spacing(8)
                    .padding(8)
                    .into(),
                SessionState::Room { wrong_file_hash: Some([[server1, server2, server3, server4, server5], [client1, client2, client3, client4, client5]]), .. } => Column::new()
                    .push("This room is for a different seed.")
                    .push(Scrollable::new(Column::new()
                        .push(Row::new()
                            .push("Room:")
                            //TODO add gray background or drop shadow in light mode
                            .push(hash_icon(server1))
                            .push(hash_icon(server2))
                            .push(hash_icon(server3))
                            .push(hash_icon(server4))
                            .push(hash_icon(server5))
                            .spacing(8)
                        )
                        .push(Row::new()
                            .push("You:")
                            //TODO add gray background or drop shadow in light mode
                            .push(hash_icon(client1))
                            .push(hash_icon(client2))
                            .push(hash_icon(client3))
                            .push(hash_icon(client4))
                            .push(hash_icon(client5))
                            .spacing(8)
                        )
                        .spacing(8)
                    ).direction(scrollable::Direction::Horizontal(scrollable::Properties::default())))
                    .push(Row::new()
                        .push(Button::new("Delete Room").on_press(Message::SetRoomView(RoomView::ConfirmDeletion)))
                        .push(Button::new("Leave Room").on_press(Message::Leave))
                        .spacing(8)
                    )
                    .spacing(8)
                    .padding(8)
                    .into(),
                SessionState::Room { wrong_file_hash: None, world_taken: Some(world), .. } => Column::new()
                    .push(Text::new(format!("World {world} is already taken.")))
                    .push(Row::new()
                        .push(Button::new("Kick").on_press(Message::Kick(world)))
                        .push(Button::new("Leave").on_press(Message::Leave))
                        .spacing(8)
                    )
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
                SessionState::Room { view: RoomView::Normal, wrong_file_hash: None, ref players, num_unassigned_clients, maintenance, .. } => {
                    let (players, other) = format_room_state(players, num_unassigned_clients, self.last_world);
                    let mut col = Column::new();
                    if let Some((start, duration)) = maintenance {
                        col = col.push(Text::new(format!(
                            "Maintenance on the Mido's House server is scheduled for {} (time shown in your local timezone). Mido's House Multiworld is expected to go offline for approximately {}.",
                            start.with_timezone(&Local).format("%A, %B %e, %H:%M"),
                            DurationFormatter(duration),
                        )));
                    }
                    col = col
                        .push(Row::new()
                            .push(Button::new("Delete Room").on_press(Message::SetRoomView(RoomView::ConfirmDeletion)))
                            .push(Button::new("Options").on_press(Message::SetRoomView(RoomView::Options)))
                            .spacing(8)
                        )
                        .push(Scrollable::new(Row::new()
                            .push(Column::with_children(players.into_iter().map(|(player_id, player)| Row::new()
                                .push(Text::new(player))
                                .push(if self.last_world.map_or(false, |my_id| my_id == player_id) {
                                    Button::new("Leave").on_press(Message::Leave)
                                } else {
                                    Button::new("Kick").on_press(Message::Kick(player_id))
                                })
                                .into()
                            ).collect()))
                            .push(Space::with_width(Length::Shrink)) // to avoid overlap with the scrollbar
                            .spacing(16)
                        ));
                    if !other.is_empty() {
                        col = col.push(Text::new(other));
                    }
                    if self.last_world.is_none() {
                        col = col.push(Button::new("Leave").on_press(Message::Leave));
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
        let mut subscriptions = Vec::with_capacity(4);
        subscriptions.push(iced::subscription::events().map(Message::Event));
        if self.updates_checked {
            match self.frontend.kind {
                Frontend::Dummy => {}
                Frontend::EverDrive => subscriptions.push(Subscription::from_recipe(everdrive::Subscription)),
                Frontend::BizHawk => if let Some(BizHawkState { port, .. }) = self.frontend.bizhawk {
                    subscriptions.push(Subscription::from_recipe(subscriptions::Connection { port, frontend: self.frontend.kind, log: self.log, connection_id: self.frontend_connection_id }));
                },
                Frontend::Pj64V3 => subscriptions.push(Subscription::from_recipe(subscriptions::Listener { frontend: self.frontend.kind, log: self.log, connection_id: self.frontend_connection_id })),
                Frontend::Pj64V4 => subscriptions.push(Subscription::from_recipe(subscriptions::Connection { port: frontend::PORT, frontend: self.frontend.kind, log: self.log, connection_id: self.frontend_connection_id })), //TODO allow Project64 to specify port via command-line arg
            }
            if !matches!(self.server_connection, SessionState::Error { .. } | SessionState::Closed) {
                subscriptions.push(Subscription::from_recipe(subscriptions::Client { log: self.log, websocket_url: self.websocket_url.clone() }));
            }
            if let SessionState::Lobby { view: LobbyView::Login { provider, no_midos_house_account: false }, .. } = self.server_connection {
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
            .push("This is a bug in Mido's House Multiworld. Please report it:")
            .push(Row::new()
                .push("• ")
                .push(Button::new("Open a GitHub issue").on_press(Message::NewIssue))
                .spacing(8)
            )
            .push("• Or post in #setup-support on the OoT Randomizer Discord. Please ping @fenhl in your message.")
            .push(Row::new()
                .push(Button::new("invite link").on_press(Message::DiscordInvite))
                .push(Button::new("direct channel link").on_press(Message::DiscordChannel))
                .spacing(8)
            )
            .push("• Or post in #general on the OoTR MW Tournament Discord.")
            .spacing(8)
            .padding(8)
        )
        .push(Space::with_width(Length::Shrink)) // to avoid overlap with the scrollbar
        .spacing(16)
    ).into()
}

#[derive(clap::Subcommand)]
#[clap(rename_all = "lower")]
enum FrontendArgs {
    #[clap(name = "dummy-frontend")]
    Dummy,
    EverDrive,
    BizHawk {
        path: PathBuf,
        pid: Pid,
        version: Version,
        port: u16,
    },
    Pj64V3,
    Pj64V4,
}

#[derive(clap::Parser)]
#[clap(version)]
struct CliArgs {
    #[clap(subcommand)]
    frontend: Option<FrontendArgs>,
}

#[wheel::main]
fn main(CliArgs { frontend }: CliArgs) -> iced::Result {
    let (icon, icon_error) = match icon::from_file_data(include_bytes!("../../../assets/icon.ico"), Some(ImageFormat::Ico)) {
        Ok(icon) => (Some(icon), None),
        Err(e) => (None, Some(e)),
    };
    State::run(Settings {
        exit_on_close_request: false,
        window: window::Settings {
            size: (360, 256),
            icon,
            ..window::Settings::default()
        },
        ..Settings::with_flags((
            icon_error,
            Config::blocking_load(),
            PersistentState::blocking_load(),
            frontend,
        ))
    })
}
