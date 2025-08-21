#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use {
    std::{
        borrow::Cow,
        collections::{
            BTreeMap,
            HashMap,
            HashSet,
        },
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
    chrono::{
        TimeDelta,
        prelude::*,
    },
    enum_iterator::all,
    futures::{
        future::{
            self,
            FutureExt as _,
        },
        sink::SinkExt as _,
        stream::Stream,
    },
    iced::{
        Element,
        Length,
        Task,
        Size,
        Subscription,
        advanced::subscription,
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
    ootr::model::{
        DungeonReward,
        Medallion,
        Stone,
    },
    ootr_utils::spoiler::HashIcon,
    open::that as open,
    rand::{
        prelude::*,
        rng,
    },
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
        sync::mpsc,
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
        HintArea,
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
        subscriptions::{
            LoggingSubscription,
            WsSink,
        },
    },
};
#[cfg(unix)] use xdg::BaseDirectories;
#[cfg(windows)] use directories::ProjectDirs;
#[cfg(target_os = "linux")] use std::os::unix::fs::PermissionsExt as _;

mod everdrive;
mod login;
mod persistent_state;
mod subscriptions;

static LOG: Lazy<Mutex<std::fs::File>> = Lazy::new(|| {
    let path = {
        #[cfg(unix)] {
            BaseDirectories::new().place_data_file("midos-house/multiworld-gui.log").expect("failed to create log dir")
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
            lock!(log = LOG; writeln!(&*log, "{} {}: {msg:?}", Utc::now().format("%Y-%m-%d %H:%M:%S"), self.context)).map_err(|e| async_proto::ReadError {
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
        let msg = ServerMessage::read_ws024(&mut self.inner).await?;
        if self.log {
            lock!(log = LOG; writeln!(&*log, "{} {}: {msg:?}", Utc::now().format("%Y-%m-%d %H:%M:%S"), self.context)).map_err(|e| async_proto::ReadError {
                context: async_proto::ErrorContext::Custom(format!("multiworld-gui::LoggingStream::read_owned")),
                kind: e.into(),
            })?;
        }
        Ok((self, msg))
    }
}

#[derive(Clone)]
struct LoggingFrontendWriter {
    log: bool,
    inner: FrontendWriter,
}

#[derive(Debug, Clone)]
enum FrontendWriter {
    Mpsc(mpsc::Sender<frontend::ServerMessage>),
    Tcp(Arc<Mutex<OwnedWriteHalf>>),
}

impl LoggingFrontendWriter {
    async fn write(&self, msg: frontend::ServerMessage) -> Result<(), Error> {
        if self.log {
            lock!(log = LOG; writeln!(&*log, "{} to frontend: {msg:?}", Utc::now().format("%Y-%m-%d %H:%M:%S"))).map_err(|e| async_proto::WriteError {
                context: async_proto::ErrorContext::Custom(format!("multiworld-gui::LoggingFrontendWriter::write")),
                kind: e.into(),
            })?;
        }
        Ok(match self.inner {
            FrontendWriter::Mpsc(ref tx) => tx.send(msg).await?,
            FrontendWriter::Tcp(ref inner) => lock!(inner = inner; msg.write(&mut *inner).await)?,
        })
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
            lock!(log = LOG; writeln!(&*log, "{} {}: {msg:?}", Utc::now().format("%Y-%m-%d %H:%M:%S"), self.context)).map_err(|e| async_proto::WriteError {
                context: async_proto::ErrorContext::Custom(format!("multiworld-gui::LoggingSink::write")),
                kind: e.into(),
            })?;
        }
        lock!(inner = self.inner; msg.write_ws024(&mut *inner).await)
    }
}

fn hash_icon(icon: HashIcon) -> Element<'static, Message> {
    match icon {
        HashIcon::Beans => Svg::new(svg::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/beans.svg")[..])).width(50).height(50).into(),
        HashIcon::BigMagic => Svg::new(svg::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/big-magic.svg")[..])).width(50).height(50).into(),
        HashIcon::Bombchu => Image::new(image::Handle::from_bytes(&include_bytes!("../../../assets/hash-icon/bombchu.png")[..])).width(50).height(50).into(),
        HashIcon::Boomerang => Svg::new(svg::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/boomerang.svg")[..])).width(50).height(50).into(),
        HashIcon::BossKey => Image::new(image::Handle::from_bytes(&include_bytes!("../../../assets/hash-icon/boss-key.png")[..])).width(50).height(50).into(),
        HashIcon::BottledFish => Image::new(image::Handle::from_bytes(&include_bytes!("../../../assets/hash-icon/bottled-fish.png")[..])).width(50).height(50).into(),
        HashIcon::BottledMilk => Image::new(image::Handle::from_bytes(&include_bytes!("../../../assets/hash-icon/bottled-milk.png")[..])).width(50).height(50).into(),
        HashIcon::Bow => Svg::new(svg::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/bow.svg")[..])).width(50).height(50).into(),
        HashIcon::Compass => Image::new(image::Handle::from_bytes(&include_bytes!("../../../assets/hash-icon/compass.png")[..])).width(50).height(50).into(),
        HashIcon::Cucco => Image::new(image::Handle::from_bytes(&include_bytes!("../../../assets/hash-icon/cucco.png")[..])).width(50).height(50).into(),
        HashIcon::DekuNut => Image::new(image::Handle::from_bytes(&include_bytes!("../../../assets/hash-icon/deku-nut.png")[..])).width(50).height(50).into(),
        HashIcon::DekuStick => Image::new(image::Handle::from_bytes(&include_bytes!("../../../assets/hash-icon/deku-stick.png")[..])).width(50).height(50).into(),
        HashIcon::FairyOcarina => Svg::new(svg::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/fairy-ocarina.svg")[..])).width(50).height(50).into(),
        HashIcon::Frog => Image::new(image::Handle::from_bytes(&include_bytes!("../../../assets/hash-icon/frog.png")[..])).width(50).height(50).into(),
        HashIcon::GoldScale => Svg::new(svg::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/gold-scale.svg")[..])).width(50).height(50).into(),
        HashIcon::HeartContainer => Image::new(image::Handle::from_bytes(&include_bytes!("../../../assets/hash-icon/heart-container.png")[..])).width(50).height(50).into(),
        HashIcon::HoverBoots => Image::new(image::Handle::from_bytes(&include_bytes!("../../../assets/hash-icon/hover-boots.png")[..])).width(50).height(50).into(),
        HashIcon::KokiriTunic => Image::new(image::Handle::from_bytes(&include_bytes!("../../../assets/hash-icon/kokiri-tunic.png")[..])).width(50).height(50).into(),
        HashIcon::LensOfTruth => Svg::new(svg::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/lens-of-truth.svg")[..])).width(50).height(50).into(),
        HashIcon::Longshot => Svg::new(svg::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/longshot.svg")[..])).width(50).height(50).into(),
        HashIcon::Map => Image::new(image::Handle::from_bytes(&include_bytes!("../../../assets/hash-icon/map.png")[..])).width(50).height(50).into(),
        HashIcon::MaskOfTruth => Svg::new(svg::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/mask-of-truth.svg")[..])).width(50).height(50).into(),
        HashIcon::MasterSword => Svg::new(svg::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/master-sword.svg")[..])).width(50).height(50).into(),
        HashIcon::MegatonHammer => Svg::new(svg::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/megaton-hammer.svg")[..])).width(50).height(50).into(),
        HashIcon::MirrorShield => Svg::new(svg::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/mirror-shield.svg")[..])).width(50).height(50).into(),
        HashIcon::Mushroom => Image::new(image::Handle::from_bytes(&include_bytes!("../../../assets/hash-icon/mushroom.png")[..])).width(50).height(50).into(),
        HashIcon::Saw => Image::new(image::Handle::from_bytes(&include_bytes!("../../../assets/hash-icon/saw.png")[..])).width(50).height(50).into(),
        HashIcon::SilverGauntlets => Svg::new(svg::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/silver-gauntlets.svg")[..])).width(50).height(50).into(),
        HashIcon::SkullToken => Svg::new(svg::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/skull-token.svg")[..])).width(50).height(50).into(),
        HashIcon::Slingshot => Svg::new(svg::Handle::from_memory(&include_bytes!("../../../assets/hash-icon/slingshot.svg")[..])).width(50).height(50).into(),
        HashIcon::SoldOut => Image::new(image::Handle::from_bytes(&include_bytes!("../../../assets/hash-icon/sold-out.png")[..])).width(50).height(50).into(),
        HashIcon::StoneOfAgony => Image::new(image::Handle::from_bytes(&include_bytes!("../../../assets/hash-icon/stone-of-agony.png")[..])).width(50).height(50).into(),
    }
}

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)] Client(#[from] multiworld::ClientError),
    #[error(transparent)] Config(#[from] multiworld::config::Error),
    #[error(transparent)] Elapsed(#[from] tokio::time::error::Elapsed),
    #[error(transparent)] EverDrive(#[from] everdrive::Error),
    #[error(transparent)] Http(#[from] tungstenite::http::Error),
    #[error(transparent)] InvalidUri(#[from] tungstenite::http::uri::InvalidUri),
    #[error(transparent)] Io(#[from] io::Error),
    #[error(transparent)] Json(#[from] serde_json::Error),
    #[error(transparent)] MpscFrontendSend(#[from] mpsc::error::SendError<frontend::ServerMessage>),
    #[error(transparent)] PersistentState(#[from] persistent_state::Error),
    #[error(transparent)] Read(#[from] async_proto::ReadError),
    #[error(transparent)] Reqwest(#[from] reqwest::Error),
    #[error(transparent)] Semver(#[from] semver::Error),
    #[error(transparent)] Url(#[from] url::ParseError),
    #[error(transparent)] WebSocket(#[from] tungstenite::Error),
    #[error(transparent)] Wheel(#[from] wheel::Error),
    #[error(transparent)] Write(#[from] async_proto::WriteError),
    #[cfg(windows)]
    #[error("user folder not found")]
    MissingHomeDir,
    #[error("Project64 script path is invalid, you can fix the script path by following the instructions defined in step 11 to 16 at:\nhttps://github.com/midoshouse/ootr-multiworld/blob/main/assets/doc/manual-install.md#for-project64\nor try to re-install Mido's House Multiworld using the installer")]
    InvalidPj64ScriptPath,
    #[error("Failed to open Project64, make sure your script path is valid by following the instructions defined in step 11 to 16 at:\nhttps://github.com/midoshouse/ootr-multiworld/blob/main/assets/doc/manual-install.md#for-project64\nor try to re-install Mido's House Multiworld using the installer")]
    Pj64LaunchFailed(#[source] io::Error),
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
            Self::Config(_) | Self::EverDrive(_) | Self::Http(_) | Self::InvalidUri(_) | Self::Json(_) | Self::MpscFrontendSend(_) | Self::PersistentState(_) | Self::Semver(_) | Self::Url(_) | Self::InvalidPj64ScriptPath | Self::VersionMismatch { .. } => false,
            Self::Client(e) => e.is_network_error(),
            Self::Io(e) | Self::Pj64LaunchFailed(e) => e.is_network_error(),
            Self::Read(e) => e.is_network_error(),
            Self::Reqwest(e) => e.is_network_error(),
            Self::WebSocket(e) => e.is_network_error(),
            Self::Wheel(e) => e.is_network_error(),
            Self::Write(e) => e.is_network_error(),
            #[cfg(windows)] Self::MissingHomeDir => false,
        }
    }
}

#[derive(Debug, Clone)]
enum Message {
    CheckForUpdates,
    CloseRequested(window::Id),
    CommandError(Arc<Error>),
    ConfirmRoomDeletion,
    CopyDebugInfo(bool),
    CreateMidosHouseAccount(login::Provider),
    DiscordChannel,
    DiscordInvite,
    DismissConflictingItemKinds,
    DismissWrongPassword,
    EverDriveScanFailed(Arc<Vec<(tokio_serial::SerialPortInfo, everdrive::ConnectError)>>),
    EverDriveTimeout,
    Exit,
    FrontendConnected(FrontendWriter),
    FrontendSubscriptionError(Arc<Error>),
    JoinRoom,
    Kick(NonZeroU8),
    LaunchProject64,
    Leave,
    LoginError(Arc<login::Error>),
    LoginTokens {
        provider: login::Provider,
        bearer_token: String,
        refresh_token: Option<String>,
    },
    NewIssue(bool),
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
    SessionExpired {
        provider: login::Provider,
        error: Option<Arc<oauth2::basic::BasicRequestTokenError<oauth2::reqwest::HttpClientError>>>,
    },
    SetAutoDeleteDelta(DurationFormatter),
    SetCreateNewRoom(bool),
    SetExistingRoomSelection(RoomFormatter),
    SetFrontend(Frontend),
    SetLobbyView(LobbyView),
    SetNewRoomName(String),
    SetPassword(String),
    SetRoomFilter(String),
    SetRoomView(RoomView),
    SetSendAllPath(String),
    SetSendAllWorld(String),
    ShowConflictingItemKindsIssue,
    ShowLoggingInstructions,
    ToggleRoomFilter,
    ToggleUpdateErrorDetails,
    UpToDate,
    #[cfg(target_os = "macos")] UpdateAvailable(Version),
    UpdateError(Arc<Error>),
}

fn cmd(future: impl Future<Output = Result<Message, Error>> + Send + 'static) -> Task<Message> {
    Task::future(
        future.map(|res| res.unwrap_or_else(|e| Message::CommandError(Arc::new(e.into()))))
    )
}

enum UpdateState {
    Pending,
    UpToDate,
    #[cfg(target_os = "macos")] Available(Version),
    Error {
        e: Arc<Error>,
        expanded: bool,
    },
}

struct State {
    persistent_state: PersistentState,
    frontend: FrontendState,
    debug_info_copied: HashSet<bool>,
    icon_error: Option<Arc<icon::Error>>,
    config_error: Option<Arc<multiworld::config::Error>>,
    persistent_state_error: Option<Arc<persistent_state::Error>>,
    command_error: Option<Arc<Error>>,
    login_error: Option<Arc<login::Error>>,
    frontend_subscription_error: Option<Arc<Error>>,
    frontend_connection_id: u8,
    frontend_writer: Option<LoggingFrontendWriter>,
    log: bool,
    pj64_script_path: Option<PathBuf>,
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
    last_hash: Option<Option<[HashIcon; 5]>>,
    last_save: Option<oottracker::Save>,
    last_dungeon_reward_locations: HashMap<DungeonReward, (NonZeroU8, HintArea)>,
    update_state: UpdateState,
    send_all_path: String,
    send_all_world: String,
    show_room_filter: bool,
    room_filter: String,
}

impl State {
    fn error_to_markdown(&self, update: bool) -> String {
        if update {
            if let UpdateState::Error { ref e, .. } = self.update_state {
                MessageBuilder::default()
                    .push_line(format!("{}error while attempting to update Mido's House Multiworld from version {}{}:",
                        if e.is_network_error() { "network " } else { "" },
                        env!("CARGO_PKG_VERSION"),
                        {
                            #[cfg(debug_assertions)] { " (debug)" }
                            #[cfg(not(debug_assertions))] { "" }
                        },
                    ))
                    .push_line_safe(e.to_string())
                    .push_codeblock_safe(format!("{e:?}"), Some("rust"))
                    .build()
            } else {
                format!("tried to copy debug info with no active error")
            }
        } else {
            if let Some(ref e) = self.icon_error {
                MessageBuilder::default()
                    .push_line(format!("error in Mido's House Multiworld version {}{} while trying to load icon:", env!("CARGO_PKG_VERSION"), {
                        #[cfg(debug_assertions)] { " (debug)" }
                        #[cfg(not(debug_assertions))] { "" }
                    }))
                    .push_line_safe(e.to_string())
                    .push_codeblock_safe(format!("{e:?}"), Some("rust"))
                    .build()
            } else if let Some(ref e) = self.config_error {
                MessageBuilder::default()
                    .push_line(format!("error in Mido's House Multiworld version {}{} while trying to load config:", env!("CARGO_PKG_VERSION"), {
                        #[cfg(debug_assertions)] { " (debug)" }
                        #[cfg(not(debug_assertions))] { "" }
                    }))
                    .push_line_safe(e.to_string())
                    .push_codeblock_safe(format!("{e:?}"), Some("rust"))
                    .build()
            } else if let Some(ref e) = self.persistent_state_error {
                MessageBuilder::default()
                    .push_line(format!("error in Mido's House Multiworld version {}{} while trying to load persistent state:", env!("CARGO_PKG_VERSION"), {
                        #[cfg(debug_assertions)] { " (debug)" }
                        #[cfg(not(debug_assertions))] { "" }
                    }))
                    .push_line_safe(e.to_string())
                    .push_codeblock_safe(format!("{e:?}"), Some("rust"))
                    .build()
            } else if let Some(ref e) = self.command_error {
                MessageBuilder::default()
                    .push_line(format!("error in Mido's House Multiworld version {}{}:", env!("CARGO_PKG_VERSION"), {
                        #[cfg(debug_assertions)] { " (debug)" }
                        #[cfg(not(debug_assertions))] { "" }
                    }))
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
                    .push_line(format!("error in Mido's House Multiworld version {}{} while trying to sign in{with_provider}:", env!("CARGO_PKG_VERSION"), {
                        #[cfg(debug_assertions)] { " (debug)" }
                        #[cfg(not(debug_assertions))] { "" }
                    }))
                    .push_line_safe(e.to_string())
                    .push_codeblock_safe(format!("{e:?}"), Some("rust"))
                    .build()
            } else if let Some(ref e) = self.frontend_subscription_error {
                MessageBuilder::default()
                    .push_line(format!("error in Mido's House Multiworld version {}{} during communication with {}:", env!("CARGO_PKG_VERSION"), {
                        #[cfg(debug_assertions)] { " (debug)" }
                        #[cfg(not(debug_assertions))] { "" }
                    }, self.frontend.display_with_version()))
                    .push_line_safe(e.to_string())
                    .push_codeblock_safe(format!("{e:?}"), Some("rust"))
                    .build()
            } else if self.frontend_writer.is_none() && self.frontend.kind != Frontend::Dummy && matches!(self.frontend.kind, Frontend::EverDrive) && matches!(self.frontend.everdrive, EverDriveState::Searching(_)) {
                let EverDriveState::Searching(ref errors) = self.frontend.everdrive else { unreachable!() };
                if errors.is_empty() {
                    MessageBuilder::default()
                        .push(format!("error in Mido's House Multiworld version {}{} while searching for EverDrives: no serial ports found", env!("CARGO_PKG_VERSION"), {
                            #[cfg(debug_assertions)] { " (debug)" }
                            #[cfg(not(debug_assertions))] { "" }
                        }))
                        .build()
                } else {
                    let mut builder = MessageBuilder::default();
                    builder.push(format!("error in Mido's House Multiworld version {}{} while searching for EverDrives:", env!("CARGO_PKG_VERSION"), {
                        #[cfg(debug_assertions)] { " (debug)" }
                        #[cfg(not(debug_assertions))] { "" }
                    }));
                    for (port, error) in &**errors {
                        builder.push_line("");
                        builder.push_mono_safe(&port.port_name);
                        builder.push_line(':');
                        builder.push_codeblock_safe(format!("{error:?}"), Some("rust"));
                    }
                    builder.build()
                }
            } else {
                match self.server_connection {
                    SessionState::Error { ref e, .. } => MessageBuilder::default()
                        .push_line(if_chain! {
                            if let SessionStateError::Connection(e) = e;
                            if e.is_network_error();
                            then {
                                format!("network error in Mido's House Multiworld version {}{}:", env!("CARGO_PKG_VERSION"), {
                                    #[cfg(debug_assertions)] { " (debug)" }
                                    #[cfg(not(debug_assertions))] { "" }
                                })
                            } else {
                                format!("error in Mido's House Multiworld version {}{} during communication with the server:", env!("CARGO_PKG_VERSION"), {
                                    #[cfg(debug_assertions)] { " (debug)" }
                                    #[cfg(not(debug_assertions))] { "" }
                                })
                            }
                        })
                        .push_line_safe(e.to_string())
                        .push_codeblock_safe(format!("{e:?}"), Some("rust"))
                        .build(),
                    SessionState::Lobby { view: LobbyView::SessionExpired { provider, error: Some(ref e) }, .. } => MessageBuilder::default()
                        .push_line(format!("error in Mido's House Multiworld version {}{} while refreshing {provider} login session:", env!("CARGO_PKG_VERSION"), {
                            #[cfg(debug_assertions)] { " (debug)" }
                            #[cfg(not(debug_assertions))] { "" }
                        }))
                        .push_line_safe(e.to_string())
                        .push_codeblock_safe(format!("{e:?}"), Some("rust"))
                        .build(),
                    _ => format!("tried to copy debug info with no active error"),
                }
            }
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[derive(Debug, Clone)]
struct BizHawkState {
    path: PathBuf,
    pid: Pid,
    version: Version,
    port: u16,
}

#[derive(Debug, Default, Clone)]
enum EverDriveState {
    #[default]
    Init,
    Searching(Arc<Vec<(tokio_serial::SerialPortInfo, everdrive::ConnectError)>>),
    Connected,
    Timeout,
}

#[derive(Debug, Clone)]
struct FrontendState {
    kind: Frontend,
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    bizhawk: Option<BizHawkState>,
    everdrive: EverDriveState,
}

impl FrontendState {
    fn display_with_version(&self) -> Cow<'static, str> {
        match self.kind {
            Frontend::Dummy => "(no frontend)".into(),
            Frontend::EverDrive => "EverDrive".into(),
            #[cfg(any(target_os = "linux", target_os = "windows"))] Frontend::BizHawk => if let Some(BizHawkState { ref version, .. }) = self.bizhawk {
                format!("BizHawk {version}").into()
            } else {
                "BizHawk".into()
            },
            #[cfg(not(any(target_os = "linux", target_os = "windows")))] Frontend::BizHawk => unreachable!("no BizHawk support on this platform"),
            Frontend::Pj64V3 => "Project64 3.x".into(),
            Frontend::Pj64V4 => "Project64 4.x".into(),
        }
    }

    fn is_locked(&self) -> bool {
        match self.kind {
            Frontend::Dummy | Frontend::EverDrive | Frontend::Pj64V3 => false,
            Frontend::Pj64V4 => false, //TODO pass port from PJ64, consider locked if present
            #[cfg(any(target_os = "linux", target_os = "windows"))] Frontend::BizHawk => self.bizhawk.is_some(),
            #[cfg(not(any(target_os = "linux", target_os = "windows")))] Frontend::BizHawk => unreachable!("no BizHawk support on this platform"),
        }
    }
}

impl State {
    fn new(icon_error: Option<icon::Error>, config: Result<Config, multiworld::config::Error>, persistent_state: Result<PersistentState, persistent_state::Error>, frontend: Option<FrontendArgs>) -> Self {
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
                None => config.default_frontend.unwrap_or({
                    #[cfg(windows)] { Frontend::Pj64V3 }
                    #[cfg(not(windows))] { Frontend::EverDrive }
                }),
                Some(FrontendArgs::Dummy) => Frontend::Dummy,
                Some(FrontendArgs::EverDrive) => Frontend::EverDrive,
                Some(FrontendArgs::BizHawk { .. }) => Frontend::BizHawk,
                Some(FrontendArgs::Pj64V3) => Frontend::Pj64V3,
                Some(FrontendArgs::Pj64V4) => Frontend::Pj64V4,
            },
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            bizhawk: if let Some(FrontendArgs::BizHawk { path, pid, version, port }) = frontend {
                Some(BizHawkState { path, pid, version, port })
            } else {
                None
            },
            everdrive: EverDriveState::default(),
        };
        Self {
            debug_info_copied: HashSet::default(),
            icon_error: icon_error.map(Arc::new),
            command_error: None,
            login_error: None,
            frontend_subscription_error: None,
            frontend_connection_id: 0,
            frontend_writer: None,
            websocket_url: config.websocket_url().expect("failed to parse WebSocket URL"),
            log: config.log,
            pj64_script_path: config.pj64_script_path,
            login_tokens: config.login_tokens,
            refresh_tokens: config.refresh_tokens,
            last_login_url: None,
            server_connection: SessionState::default(),
            server_writer: None,
            retry: Instant::now(),
            wait_time: Duration::from_secs(1),
            last_world: None,
            last_name: Filename::default(),
            last_hash: None,
            last_save: None,
            last_dungeon_reward_locations: HashMap::default(),
            update_state: UpdateState::Pending,
            send_all_path: String::default(),
            send_all_world: String::default(),
            show_room_filter: false,
            room_filter: String::default(),
            frontend, config_error, persistent_state_error, persistent_state,
        }
    }

    fn title(&self) -> String {
        if self.frontend.is_locked() {
            format!("Mido's House Multiworld for {}", self.frontend.kind)
        } else {
            format!("Mido's House Multiworld")
        }
    }

    fn update(&mut self, msg: Message) -> Task<Message> {
        match msg {
            Message::SetLobbyView(new_view) => if let SessionState::Lobby { ref mut view, .. } = self.server_connection {
                *view = new_view;
            },
            Message::SetRoomView(new_view) => if let SessionState::Room { ref mut view, .. } = self.server_connection {
                *view = new_view;
            },
            Message::CheckForUpdates => {
                self.update_state = UpdateState::Pending;
                #[cfg(any(target_os = "linux", target_os = "windows"))] let frontend = self.frontend.clone();
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
                            #[cfg(any(target_os = "linux", target_os = "windows"))] {
                                let updater_path = {
                                    #[cfg(unix)] {
                                        BaseDirectories::new().place_cache_file("midos-house/multiworld-updater")?
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
                                let mut cmd = process::Command::new(updater_path);
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
                            #[cfg(target_os = "macos")] {
                                return Ok(Message::UpdateAvailable(new_ver))
                            }
                        }
                    }
                    Ok(Message::UpToDate)
                }.map(|res| Ok(res.unwrap_or_else(|e| Message::UpdateError(Arc::new(e))))))
            }
            Message::CloseRequested(window) => if self.command_error.is_some() || self.login_error.is_some() || self.frontend_subscription_error.is_some() {
                return window::close(window)
            } else {
                let frontend_writer = self.frontend_writer.take();
                let server_writer = self.server_writer.take();
                return cmd(async move {
                    if let Some(frontend_writer) = frontend_writer {
                        if let FrontendWriter::Tcp(writer) = frontend_writer.inner {
                            lock!(writer = writer; writer.shutdown().await)?;
                        }
                    }
                    if let Some(server_writer) = server_writer {
                        lock!(server_writer = server_writer.inner; {
                            server_writer.send(tungstenite::Message::Close(Some(tungstenite::protocol::CloseFrame {
                                code: tungstenite::protocol::frame::coding::CloseCode::Away,
                                reason: "multiworld app exiting".into(),
                            }))).await?;
                            server_writer.close().await?;
                        });
                    }
                    Ok(Message::Exit)
                })
            },
            Message::CommandError(e) => { self.command_error.get_or_insert(e); }
            Message::ConfirmRoomDeletion => if let Some(writer) = self.server_writer.clone() {
                return cmd(async move {
                    writer.write(ClientMessage::DeleteRoom).await?;
                    Ok(Message::Nop)
                })
            },
            Message::CopyDebugInfo(update) => {
                let error_md = self.error_to_markdown(update);
                self.debug_info_copied.insert(update);
                return clipboard::write(error_md)
            }
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
            Message::DismissConflictingItemKinds => if let SessionState::Room { ref mut conflicting_item_kinds, .. } = self.server_connection {
                *conflicting_item_kinds = false;
            },
            Message::DismissWrongPassword => if let SessionState::Lobby { ref mut wrong_password, .. } = self.server_connection {
                *wrong_password = false;
            },
            Message::EverDriveScanFailed(errors) => {
                self.frontend.everdrive = EverDriveState::Searching(errors);
                if let Frontend::EverDrive = self.frontend.kind {
                    self.frontend_writer = None;
                }
            }
            Message::EverDriveTimeout => {
                self.frontend.everdrive = EverDriveState::Timeout;
                if let Frontend::EverDrive = self.frontend.kind {
                    self.frontend_writer = None;
                }
            }
            Message::Exit => return iced::exit(),
            Message::FrontendConnected(inner) => {
                if let Frontend::EverDrive = self.frontend.kind {
                    self.frontend.everdrive = EverDriveState::Connected;
                }
                let writer = LoggingFrontendWriter { log: self.log, inner };
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
                        (Frontend::BizHawk | Frontend::Pj64V4, io::ErrorKind::ConnectionReset | io::ErrorKind::UnexpectedEof) => return iced::exit(), // frontend closed
                        (Frontend::Pj64V3, io::ErrorKind::ConnectionReset) => {
                            self.frontend_writer = None;
                            return Task::none()
                        }
                        (_, _) => {}
                    }
                }
                self.frontend_subscription_error.get_or_insert(e);
            }
            Message::ToggleRoomFilter => {
                self.show_room_filter = !self.show_room_filter;
                if self.show_room_filter {
                    return text_input::focus("room-filter")
                }
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
            Message::NewIssue(update) => {
                let mut issue_url = match Url::parse("https://github.com/midoshouse/ootr-multiworld/issues/new") {
                    Ok(issue_url) => issue_url,
                    Err(e) => return cmd(future::err(e.into())),
                };
                issue_url.query_pairs_mut().append_pair("body", &self.error_to_markdown(update));
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
                        if let Some(hash) = self.last_hash {
                            let persistent_state = self.persistent_state.clone();
                            return cmd(async move {
                                persistent_state.edit(move |state| state.pending_items_after_save.push(persistent_state::PendingItem { hash, key, kind, target_world })).await?;
                                Ok(Message::Nop)
                            })
                        }
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
                frontend::ClientMessage::DungeonRewardInfo { emerald, ruby, sapphire, light, forest, fire, water, shadow, spirit } => {
                    let mut messages = Vec::default();
                    if let Some((world, area)) = emerald {
                        if self.last_dungeon_reward_locations.insert(DungeonReward::Stone(Stone::KokiriEmerald), (world, area)) != Some((world, area)) {
                            messages.push(ClientMessage::DungeonRewardInfo { reward: DungeonReward::Stone(Stone::KokiriEmerald), world, area });
                        }
                    }
                    if let Some((world, area)) = ruby {
                        if self.last_dungeon_reward_locations.insert(DungeonReward::Stone(Stone::GoronRuby), (world, area)) != Some((world, area)) {
                            messages.push(ClientMessage::DungeonRewardInfo { reward: DungeonReward::Stone(Stone::GoronRuby), world, area });
                        }
                    }
                    if let Some((world, area)) = sapphire {
                        if self.last_dungeon_reward_locations.insert(DungeonReward::Stone(Stone::ZoraSapphire), (world, area)) != Some((world, area)) {
                            messages.push(ClientMessage::DungeonRewardInfo { reward: DungeonReward::Stone(Stone::ZoraSapphire), world, area });
                        }
                    }
                    if let Some((world, area)) = light {
                        if self.last_dungeon_reward_locations.insert(DungeonReward::Medallion(Medallion::Light), (world, area)) != Some((world, area)) {
                            messages.push(ClientMessage::DungeonRewardInfo { reward: DungeonReward::Medallion(Medallion::Light), world, area });
                        }
                    }
                    if let Some((world, area)) = forest {
                        if self.last_dungeon_reward_locations.insert(DungeonReward::Medallion(Medallion::Forest), (world, area)) != Some((world, area)) {
                            messages.push(ClientMessage::DungeonRewardInfo { reward: DungeonReward::Medallion(Medallion::Forest), world, area });
                        }
                    }
                    if let Some((world, area)) = fire {
                        if self.last_dungeon_reward_locations.insert(DungeonReward::Medallion(Medallion::Fire), (world, area)) != Some((world, area)) {
                            messages.push(ClientMessage::DungeonRewardInfo { reward: DungeonReward::Medallion(Medallion::Fire), world, area });
                        }
                    }
                    if let Some((world, area)) = water {
                        if self.last_dungeon_reward_locations.insert(DungeonReward::Medallion(Medallion::Water), (world, area)) != Some((world, area)) {
                            messages.push(ClientMessage::DungeonRewardInfo { reward: DungeonReward::Medallion(Medallion::Water), world, area });
                        }
                    }
                    if let Some((world, area)) = shadow {
                        if self.last_dungeon_reward_locations.insert(DungeonReward::Medallion(Medallion::Shadow), (world, area)) != Some((world, area)) {
                            messages.push(ClientMessage::DungeonRewardInfo { reward: DungeonReward::Medallion(Medallion::Shadow), world, area });
                        }
                    }
                    if let Some((world, area)) = spirit {
                        if self.last_dungeon_reward_locations.insert(DungeonReward::Medallion(Medallion::Spirit), (world, area)) != Some((world, area)) {
                            messages.push(ClientMessage::DungeonRewardInfo { reward: DungeonReward::Medallion(Medallion::Spirit), world, area });
                        }
                    }
                    if let Some(ref writer) = self.server_writer {
                        if let SessionState::Room { .. } = self.server_connection {
                            let writer = writer.clone();
                            return cmd(async move {
                                for message in messages {
                                    writer.write(message).await?;
                                }
                                Ok(Message::Nop)
                            })
                        }
                    }
                }
                frontend::ClientMessage::CurrentScene(scene) => if let Some(ref writer) = self.server_writer {
                    let writer = writer.clone();
                    return cmd(async move {
                        writer.write(ClientMessage::CurrentScene(scene)).await?;
                        Ok(Message::Nop)
                    })
                },
            },
            Message::ReconnectFrontend => {
                self.frontend_subscription_error = None;
                self.frontend_connection_id = self.frontend_connection_id.wrapping_add(1);
            }
            Message::ReconnectToLobby => self.server_connection = SessionState::Init { maintenance: self.server_connection.maintenance() },
            Message::ReconnectToRoom(room_id, room_password) => self.server_connection = SessionState::InitAutoRejoin { room_id, room_password, maintenance: self.server_connection.maintenance() },
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
                                let mut config = Config::load().await?;
                                config.login_tokens.remove(&login::Provider::Discord);
                                config.refresh_tokens.remove(&login::Provider::Discord);
                                config.save().await?;
                                Ok(match login::oauth_client(login::Provider::Discord)?
                                    .exchange_refresh_token(&RefreshToken::new(refresh_token))
                                    .request_async(async_http_client).await
                                {
                                    Ok(tokens) => Message::LoginTokens {
                                        provider: login::Provider::Discord,
                                        bearer_token: tokens.access_token().secret().clone(),
                                        refresh_token: tokens.refresh_token().map(|refresh_token| refresh_token.secret().clone()),
                                    },
                                    Err(e) => Message::SessionExpired {
                                        provider: login::Provider::Discord,
                                        error: Some(Arc::new(e)),
                                    },
                                })
                            })
                        } else {
                            return cmd(future::ok(Message::SessionExpired {
                                provider: login::Provider::Discord,
                                error: None,
                            }))
                        }
                    }
                    ServerMessage::StructuredError(ServerError::SessionExpiredRaceTime) => {
                        self.login_tokens.remove(&login::Provider::RaceTime);
                        if let Some(refresh_token) = self.refresh_tokens.remove(&login::Provider::RaceTime) {
                            return cmd(async move {
                                let mut config = Config::load().await?;
                                config.login_tokens.remove(&login::Provider::RaceTime);
                                config.refresh_tokens.remove(&login::Provider::RaceTime);
                                config.save().await?;
                                Ok(match login::oauth_client(login::Provider::RaceTime)?
                                    .exchange_refresh_token(&RefreshToken::new(refresh_token))
                                    .request_async(async_http_client).await
                                {
                                    Ok(tokens) => Message::LoginTokens {
                                        provider: login::Provider::RaceTime,
                                        bearer_token: tokens.access_token().secret().clone(),
                                        refresh_token: tokens.refresh_token().map(|refresh_token| refresh_token.secret().clone()),
                                    },
                                    Err(e) => Message::SessionExpired {
                                        provider: login::Provider::RaceTime,
                                        error: Some(Arc::new(e)),
                                    },
                                })
                            })
                        } else {
                            return cmd(future::ok(Message::SessionExpired {
                                provider: login::Provider::RaceTime,
                                error: None,
                            }))
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
                        let dungeon_reward_locations = self.last_dungeon_reward_locations.clone();
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
                            for persistent_state::PendingItem { hash, key, kind, target_world } in pending_items_before_save {
                                if let Some(last_hash) = file_hash {
                                    if hash != last_hash { continue }
                                }
                                server_writer.write(ClientMessage::SendItem { key, kind, target_world }).await?;
                            }
                            if let Some(save) = save {
                                server_writer.write(ClientMessage::SaveData(save)).await?;
                            }
                            for (reward, (world, area)) in dungeon_reward_locations {
                                server_writer.write(ClientMessage::DungeonRewardInfo { reward, world, area }).await?;
                            }
                            for persistent_state::PendingItem { hash, key, kind, target_world } in pending_items_after_save {
                                if let Some(last_hash) = file_hash {
                                    if hash != last_hash { continue }
                                }
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
                        self.wait_time = self.wait_time.mul_f64(rng().random_range(1.0..=2.0)); // randomized exponential backoff
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
                            return cmd(future::ok(Message::CheckForUpdates))
                        }
                    }
                }
            },
            Message::SessionExpired { provider, error } => if let SessionState::Lobby { ref mut view, .. } = self.server_connection {
                *view = LobbyView::SessionExpired { provider, error };
            },
            Message::SetAutoDeleteDelta(DurationFormatter(new_delta)) => if let Some(writer) = self.server_writer.clone() {
                return cmd(async move {
                    writer.write(ClientMessage::AutoDeleteDelta(new_delta)).await?;
                    Ok(Message::Nop)
                })
            },
            Message::SetCreateNewRoom(new_val) => if let SessionState::Lobby { ref mut create_new_room, .. } = self.server_connection { *create_new_room = new_val },
            Message::SetExistingRoomSelection(room) => {
                if room.is_dummy {
                    self.room_filter = String::default();
                } else {
                    if let SessionState::Lobby { ref mut existing_room_selection, .. } = self.server_connection { *existing_room_selection = Some(room) };
                }
                self.show_room_filter = false;
            },
            Message::SetFrontend(new_frontend) => self.frontend.kind = new_frontend,
            Message::SetNewRoomName(name) => if let SessionState::Lobby { ref mut new_room_name, .. } = self.server_connection { *new_room_name = name },
            Message::SetPassword(new_password) => if let SessionState::Lobby { ref mut password, .. } = self.server_connection { *password = new_password },
            Message::SetRoomFilter(new_room_filter) => self.room_filter = new_room_filter,
            Message::SetSendAllPath(new_path) => self.send_all_path = new_path,
            Message::SetSendAllWorld(new_world) => self.send_all_world = new_world,
            Message::ShowConflictingItemKindsIssue => if let Err(e) = open("https://github.com/midoshouse/ootr-multiworld/issues/43") {
                return cmd(future::err(e.into()))
            },
            Message::ShowLoggingInstructions => if let Err(e) = open({
                #[cfg(target_os = "windows")] { "https://github.com/midoshouse/ootr-multiworld/blob/main/assets/doc/logging-windows.md" }
                #[cfg(target_os = "linux")] { "https://github.com/midoshouse/ootr-multiworld/blob/main/assets/doc/logging-linux.md" }
                #[cfg(not(any(target_os = "windows", target_os = "linux")))] { "https://github.com/midoshouse/ootr-multiworld/blob/main/assets/doc/logging.md" }
            }) {
                return cmd(future::err(e.into()))
            },
            Message::LaunchProject64 => {
                let emulator_path = self.pj64_script_path.as_ref().expect("emulator path must be set for Project64 version 3");
                let Some(pj64_folder_path) = Path::new(emulator_path).ancestors().nth(2) else {
                    return cmd(future::err(Error::InvalidPj64ScriptPath))
                };
                let pj64_executable_path = pj64_folder_path.join("Project64.exe");
                if let Err(e) = process::Command::new(pj64_executable_path).current_dir(pj64_folder_path).spawn() {
                    return cmd(future::err(Error::Pj64LaunchFailed(e)))
                }
            }
            Message::ToggleUpdateErrorDetails => if let UpdateState::Error { ref mut expanded, .. } = self.update_state { *expanded = !*expanded },
            Message::UpToDate => self.update_state = UpdateState::UpToDate,
            #[cfg(target_os = "macos")] Message::UpdateAvailable(new_ver) => self.update_state = UpdateState::Available(new_ver),
            Message::UpdateError(e) => self.update_state = UpdateState::Error { e, expanded: false },
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let mut suppress_scroll = false;
        let main_view = if let Some(ref e) = self.icon_error {
            error_view("An error occurred:", e, false, self.debug_info_copied.contains(&false))
        } else if let Some(ref e) = self.config_error {
            error_view("An error occurred:", e, false, self.debug_info_copied.contains(&false))
        } else if let Some(ref e) = self.persistent_state_error {
            error_view("An error occurred:", e, false, self.debug_info_copied.contains(&false))
        } else if let Some(ref e) = self.command_error {
            error_view("An error occurred:", e, false, self.debug_info_copied.contains(&false))
        } else if let Some(ref e) = self.login_error {
            error_view("An error occurred while trying to sign in:", e, false, self.debug_info_copied.contains(&false)) //TODO button to reset error state
        } else if let Some(ref e) = self.frontend_subscription_error {
            if let Error::Io(ref e) = **e {
                if e.kind() == io::ErrorKind::AddrInUse {
                    Column::new()
                        .push(Text::new("Connection Busy").size(24))
                        .push(Text::new(format!("Could not connect to {} because the connection is already in use. Maybe you still have another instance of this app open?", self.frontend.kind)))
                        .push(Button::new("Retry").on_press(Message::ReconnectFrontend))
                        .spacing(8)
                        .into()
                } else {
                    error_view(format!("An error occurred during communication with {}:", self.frontend.kind), e, false, self.debug_info_copied.contains(&false))
                }
            } else {
                error_view(format!("An error occurred during communication with {}:", self.frontend.kind), e, false, self.debug_info_copied.contains(&false))
            }
        } else if let UpdateState::Pending = self.update_state {
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
            col.spacing(8)
        } else if self.frontend_writer.is_none() && self.frontend.kind != Frontend::Dummy {
            let mut col = Column::new();
            if !self.frontend.is_locked() {
                col = col.push(PickList::new(all::<Frontend>().filter(|&iter_frontend| self.frontend.kind == iter_frontend || iter_frontend.is_supported()).collect_vec(), Some(self.frontend.kind), Message::SetFrontend));
            }
            match self.frontend.kind {
                Frontend::Dummy => unreachable!(),
                Frontend::EverDrive => match self.frontend.everdrive {
                    EverDriveState::Init => col = col.push("Looking for EverDrives…"),
                    EverDriveState::Searching(ref errors) => {
                        col = col.push("Looking for EverDrives…");
                        if errors.is_empty() {
                            col = col
                                .push("No USB devices found.")
                                .push("Make sure your console is turned on and connected, and your USB cable supports data.");
                        } else if errors.iter().any(|(_, error)| matches!(error, everdrive::ConnectError::MainMenu)) {
                            col = col.push("Connected to EverDrive main menu. Please start the game.");
                        } else {
                            col = col
                                .push("Some USB devices were found but they all reported errors.")
                                .push("Make sure your console is turned on and connected, and your USB cable supports data.")
                                .push(Row::new()
                                    .push(Button::new("Copy debug info").on_press(Message::CopyDebugInfo(false)))
                                    .push(if self.debug_info_copied.contains(&false) { "Copied!" } else { "for pasting into Discord" })
                                    .align_y(iced::Alignment::Center)
                                    .spacing(8)
                                );
                        }
                    }
                    EverDriveState::Connected => col = col
                        .push("Waiting for EverDrive…")
                        .push("This should take less than 5 seconds."),
                    EverDriveState::Timeout => col = col
                        .push("Connection to EverDrive lost")
                        .push("Retrying in 5 seconds…"),
                },
                #[cfg(any(target_os = "linux", target_os = "windows"))] Frontend::BizHawk => if self.frontend.bizhawk.is_some() {
                    col = col
                        .push("Waiting for BizHawk…")
                        .push("Make sure your game is running and unpaused.");
                } else {
                    col = col
                        .push("BizHawk not connected")
                        .push("To use multiworld with BizHawk, start it from BizHawk's Tools → External Tools menu.");
                },
                #[cfg(not(any(target_os = "linux", target_os = "windows")))] Frontend::BizHawk => unreachable!("no BizHawk support on this platform"),
                Frontend::Pj64V3 => {
                    col = col.push("Waiting for Project64…");
                    if self.pj64_script_path.is_some() {
                        col = col
                            .push(Row::new()
                                .push("1. ")
                                .push(Button::new("Open Project64").on_press(Message::LaunchProject64))
                                .align_y(iced::Alignment::Center)
                            )
                            .push("2. In Project64's Debugger menu, select Scripts\n3. In the Scripts window, select ootrmw.js and click Run\n4. Wait until the Output area says “Connected to multiworld app”. (This should take less than 5 seconds.) You can then close the Scripts window.");
                    } else {
                        col = col.push("1. Open Project64\n2. In Project64's Debugger menu, select Scripts\n3. In the Scripts window, select ootrmw.js and click Run\n4. Wait until the Output area says “Connected to multiworld app”. (This should take less than 5 seconds.) You can then close the Scripts window.");
                    }
                }
                Frontend::Pj64V4 => {
                    col = col
                        .push("Waiting for Project64…")
                        .push("This should take less than 5 seconds.");
                }
            }
            col.spacing(8)
        } else {
            match self.server_connection {
                SessionState::Error { auto_retry: false, ref e, maintenance: _ } => error_view("An error occurred during communication with the server:", e, false, self.debug_info_copied.contains(&false)),
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
                        .push(Text::new(if let Ok(retry) = TimeDelta::from_std(self.retry.duration_since(Instant::now())) {
                            format!("Reconnecting at {}", (Local::now() + retry).format("%H:%M:%S"))
                        } else {
                            format!("Reconnecting…")
                        })) //TODO live countdown
                        .push("If this error persists, check your internet connection or contact @fenhl on Discord for support.")
                        .push(Row::new()
                            .push(Button::new("Copy debug info").on_press(Message::CopyDebugInfo(false)))
                            .push(if self.debug_info_copied.contains(&false) { "Copied!" } else { "for pasting into Discord" })
                            .align_y(iced::Alignment::Center)
                            .spacing(8)
                        )
                        .spacing(8)
                }
                SessionState::Init { maintenance } => {
                    let mut col = Column::new();
                    if let Some((start, duration)) = maintenance {
                        col = col.push(Text::new(format!(
                            "Maintenance on the Mido's House server is scheduled for {} (time shown in your local timezone). Mido's House Multiworld is expected to go offline for approximately {}.",
                            start.with_timezone(&Local).format("%A, %B %e, %H:%M"),
                            DurationFormatter(duration),
                        )));
                    }
                    col
                        .push("Connecting to server…")
                        .push("If this takes longer than 5 seconds, check your internet connection or contact @fenhl on Discord for support.")
                        .push({ suppress_scroll = true; Space::with_height(Length::Fill) })
                        .push(Text::new(format!("version {}{}", env!("CARGO_PKG_VERSION"), {
                            #[cfg(debug_assertions)] { " (debug)" }
                            #[cfg(not(debug_assertions))] { "" }
                        })))
                        .spacing(8)
                }
                SessionState::InitAutoRejoin { .. } => Column::new()
                    .push("Reconnecting to room…")
                    .push("If this takes longer than 5 seconds, check your internet connection or contact @fenhl on Discord for support.")
                    .push({ suppress_scroll = true; Space::with_height(Length::Fill) })
                    .push(Text::new(format!("version {}{}", env!("CARGO_PKG_VERSION"), {
                        #[cfg(debug_assertions)] { " (debug)" }
                        #[cfg(not(debug_assertions))] { "" }
                    })))
                    .spacing(8),
                SessionState::Lobby { wrong_password: true, .. } => Column::new()
                    .push("wrong password")
                    .push({ suppress_scroll = true; Space::with_height(Length::Fill) })
                    .push(Button::new("OK").on_press(Message::DismissWrongPassword))
                    .spacing(8),
                SessionState::Lobby { view: LobbyView::SessionExpired { provider, error: None }, wrong_password: false, .. } => Column::new()
                    .push(Text::new(format!("Your Mido's House user session has expired.")))
                    .push(Button::new("Sign back in").on_press(Message::SetLobbyView(LobbyView::Login { provider, no_midos_house_account: false })))
                    .push(Space::with_width(Length::Fill))
                    .push(Button::new("Cancel").on_press(Message::SetLobbyView(LobbyView::Normal)))
                    .spacing(8),
                SessionState::Lobby { view: LobbyView::SessionExpired { provider, error: Some(ref e) }, wrong_password: false, .. } => Column::new()
                    .push(Text::new(format!("Failed to refresh your Mido's House user session:")))
                    .push(Text::new(e.to_string()))
                    .push(Button::new("Sign back in").on_press(Message::SetLobbyView(LobbyView::Login { provider, no_midos_house_account: false })))
                    .push("If this error persists, contact @fenhl on Discord for support.")
                    .push(Row::new()
                        .push(Button::new("Copy debug info").on_press(Message::CopyDebugInfo(false)))
                        .push(if self.debug_info_copied.contains(&false) { "Copied!" } else { "for pasting into Discord" })
                        .align_y(iced::Alignment::Center)
                        .spacing(8)
                    )
                    .push(Space::with_width(Length::Fill))
                    .push(Button::new("Cancel").on_press(Message::SetLobbyView(LobbyView::Normal)))
                    .spacing(8),
                SessionState::Lobby { view: LobbyView::Settings, wrong_password: false, login_state, .. } => {
                    let mut col = Column::new()
                        .push(Row::new()
                            .push(Button::new("Back").on_press(Message::SetLobbyView(LobbyView::Normal)))
                            .push(Space::with_width(Length::Fill))
                            .push(Text::new(format!("version {}{}", env!("CARGO_PKG_VERSION"), {
                                #[cfg(debug_assertions)] { " (debug)" }
                                #[cfg(not(debug_assertions))] { "" }
                            })))
                            .align_y(iced::Alignment::Center)
                        );
                    if login_state.is_some() {
                        col = col.push("You are signed in."); //TODO option to sign out
                    } else {
                        col = col
                            .push("To access official tournament rooms, sign into Mido's House:")
                            .push(Button::new("Sign in with racetime.gg").on_press(Message::SetLobbyView(LobbyView::Login { provider: login::Provider::RaceTime, no_midos_house_account: false })))
                            .push(Button::new("Sign in with Discord").on_press(Message::SetLobbyView(LobbyView::Login { provider: login::Provider::Discord, no_midos_house_account: false })));
                    }
                    col.spacing(8)
                }
                SessionState::Lobby { view: LobbyView::Login { provider, no_midos_house_account: true }, wrong_password: false, .. } => Column::new()
                    .push(Text::new(format!("This {provider} account is not associated with a Mido's House account.")))
                    .push(Row::new()
                        .push(Button::new("Create a Mido's House account").on_press(Message::CreateMidosHouseAccount(provider)))
                        .push(",")
                        .align_y(iced::Alignment::Center)
                        .spacing(8)
                    )
                    .push(Row::new()
                        .push("then")
                        .push(Button::new("try again").on_press(Message::SetLobbyView(LobbyView::Login { provider, no_midos_house_account: false })))
                        .push(".")
                        .align_y(iced::Alignment::Center)
                        .spacing(8)
                    )
                    .push({ suppress_scroll = true; Space::with_height(Length::Fill) })
                    .push(Button::new("Cancel").on_press(Message::SetLobbyView(LobbyView::Settings)))
                    .spacing(8),
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
                    .push({ suppress_scroll = true; Space::with_height(Length::Fill) })
                    .push(Button::new("Cancel").on_press(Message::SetLobbyView(LobbyView::Settings)))
                    .spacing(8),
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
                                let mut rooms = rooms.iter()
                                    .map(|(&id, (name, password_required))| RoomFormatter { id, name: name.clone(), password_required: password_required.clone(), is_dummy: false })
                                    .filter(|room| room.name.to_lowercase().contains(&self.room_filter.to_lowercase()))
                                    .collect_vec();
                                rooms.sort_unstable();
                                if rooms.is_empty() {
                                    rooms.push(RoomFormatter { password_required: false, name: String::from("No rooms found"), id: 0, is_dummy: true });
                                }
                                let mut stack = Stack::new().width(360.0);
                                stack = stack.push(PickList::new(rooms, existing_room_selection.clone(), Message::SetExistingRoomSelection).placeholder("Select a room").on_open(Message::ToggleRoomFilter).on_close(Message::ToggleRoomFilter));
                                if self.show_room_filter {
                                    stack = stack.push(TextInput::new("Room name", &self.room_filter).on_input(Message::SetRoomFilter).on_paste(Message::SetRoomFilter).id("room-filter"));
                                }
                                stack.into()
                            }
                        });
                    if existing_room_selection.as_ref().map_or(true, |existing_room_selection| existing_room_selection.password_required) {
                        col = col.push(TextInput::new("Password", password).secure(true).on_input(Message::SetPassword).on_paste(Message::SetPassword).on_submit(Message::JoinRoom).padding(5));
                    }
                    col = col.push({ suppress_scroll = true; Space::with_height(Length::Fill) });
                    if create_new_room {
                        if new_room_name.chars().count() > 64 {
                            col = col.push("room name too long (maximum 64 characters)");
                        }
                        if new_room_name.contains('\0') {
                            col = col.push("room name must not contain null characters");
                        }
                        if password.chars().count() > 64 {
                            col = col.push("room password too long (maximum 64 characters)");
                        }
                        if password.contains('\0') {
                            col = col.push("room password must not contain null characters");
                        }
                    }
                    col
                        .push(Row::new()
                            .push({
                                let mut btn = Button::new("Connect");
                                let enabled = if create_new_room {
                                    !new_room_name.is_empty()
                                    && new_room_name.chars().count() <= 64
                                    && !new_room_name.contains('\0')
                                    && !password.is_empty()
                                    && password.chars().count() <= 64
                                    && !password.contains('\0')
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
                }
                SessionState::Room { view: RoomView::ConfirmDeletion, .. } => Column::new()
                    .push("Are you sure you want to delete this room? Items that have already been sent will be lost forever!")
                    .push(Button::new("Delete").on_press(Message::ConfirmRoomDeletion))
                    .push({ suppress_scroll = true; Space::with_height(Length::Fill) })
                    .push(Button::new("Back").on_press(Message::SetRoomView(RoomView::Normal)))
                    .spacing(8),
                SessionState::Room { conflicting_item_kinds: true, .. } => {
                    let mut col = Column::new()
                        .push("Your game sent multiple different items from the same location.")
                        .push("One of those items may now be lost. This is a known bug that's currently being investigated.");
                    if self.log {
                        col = col
                            .push("If you've had logging enabled for the entire seed, please ping @fenhl in #setup-support on the OoT Randomizer Discord to help with the investigation.")
                            .push(Row::new()
                                .push(Button::new("invite link").on_press(Message::DiscordInvite))
                                .push(Button::new("direct channel link").on_press(Message::DiscordChannel))
                                .spacing(8)
                            );
                    } else {
                        col = col
                            .push("To help with the investigation, please consider enabling logging in case you encounter the bug again in a new seed.")
                            .push(Button::new("How to enable logging").on_press(Message::ShowLoggingInstructions));
                    }
                    col
                        .push(Button::new("More info").on_press(Message::ShowConflictingItemKindsIssue))
                        .push({ suppress_scroll = true; Space::with_height(Length::Fill) })
                        .push(Button::new("Dismiss").on_press(Message::DismissConflictingItemKinds))
                        .spacing(8)
                }
                SessionState::Room { conflicting_item_kinds: false, wrong_file_hash: Some([server, client]), .. } => Column::new()
                    .push("This room is for a different seed.")
                    .push(Scrollable::new(Column::new()
                        .push(if let Some([server1, server2, server3, server4, server5]) = server {
                            Row::new()
                                .push("Room:")
                                //TODO add gray background or drop shadow in light mode
                                .push(hash_icon(server1))
                                .push(hash_icon(server2))
                                .push(hash_icon(server3))
                                .push(hash_icon(server4))
                                .push(hash_icon(server5))
                                .align_y(iced::Alignment::Center)
                                .spacing(8)
                                .into()
                        } else {
                            Element::from("Room: (old randomizer version)")
                        })
                        .push(if let Some([client1, client2, client3, client4, client5]) = client {
                            Row::new()
                                .push("You:")
                                //TODO add gray background or drop shadow in light mode
                                .push(hash_icon(client1))
                                .push(hash_icon(client2))
                                .push(hash_icon(client3))
                                .push(hash_icon(client4))
                                .push(hash_icon(client5))
                                .align_y(iced::Alignment::Center)
                                .spacing(8)
                                .into()
                        } else {
                            Element::from("You: (old randomizer version)")
                        })
                        .spacing(8)
                    ).direction(scrollable::Direction::Horizontal(scrollable::Scrollbar::default())))
                    .push(Row::new()
                        .push(Button::new("Delete Room").on_press(Message::SetRoomView(RoomView::ConfirmDeletion)))
                        .push(Button::new("Leave Room").on_press(Message::Leave))
                        .spacing(8)
                    )
                    .spacing(8),
                SessionState::Room { conflicting_item_kinds: false, wrong_file_hash: None, world_taken: Some(world), .. } => Column::new()
                    .push(Text::new(format!("World {world} is already taken.")))
                    .push(Row::new()
                        .push(Button::new("Kick").on_press(Message::Kick(world)))
                        .push(Button::new("Leave").on_press(Message::Leave))
                        .spacing(8)
                    )
                    .spacing(8),
                SessionState::Room { view: RoomView::Options, wrong_file_hash: None, autodelete_delta, allow_send_all, .. } => {
                    let mut col = Column::new()
                        .push(Button::new("Back").on_press(Message::SetRoomView(RoomView::Normal)))
                        .push(Rule::horizontal(1))
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
                        col = col
                            .push(Rule::horizontal(1))
                            .push(Row::new()
                                .push("Send all items from world:")
                                .push({
                                    let mut input = TextInput::new("", &self.send_all_world).on_input(Message::SetSendAllWorld).on_paste(Message::SetSendAllWorld).width(Length::Fixed(32.0));
                                    if self.send_all_world.parse::<NonZeroU8>().is_ok() {
                                        input = input.on_submit(Message::SendAll);
                                    }
                                    input
                                })
                                .align_y(iced::Alignment::Center)
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
                                .align_y(iced::Alignment::Center)
                                .spacing(8)
                            );
                    }
                    col.spacing(8)
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
                        .push(Column::with_children(players.into_iter().map(|(player_id, player)| Row::new()
                            .push(Text::new(player))
                            .push(if self.last_world.map_or(false, |my_id| my_id == player_id) {
                                Button::new("Leave").on_press(Message::Leave)
                            } else {
                                Button::new("Kick").on_press(Message::Kick(player_id))
                            })
                            .align_y(iced::Alignment::Center)
                            .into()
                        ).collect_vec()));
                    if !other.is_empty() {
                        col = col.push(Text::new(other));
                    }
                    if self.last_world.is_none() {
                        col = col.push(Button::new("Leave").on_press(Message::Leave));
                    }
                    col.spacing(8)
                }
                SessionState::Closed { maintenance } => {
                    let mut col = Column::new();
                    if let Some((start, duration)) = maintenance {
                        col = col.push(Text::new(format!(
                            "Maintenance on the Mido's House server is scheduled for {} (time shown in your local timezone). Mido's House Multiworld is expected to go offline for approximately {}.",
                            start.with_timezone(&Local).format("%A, %B %e, %H:%M"),
                            DurationFormatter(duration),
                        )));
                    }
                    col
                        .push("You have been disconnected.")
                        .push(Button::new("Reconnect").on_press(Message::ReconnectToLobby))
                        .spacing(8)
                }
            }
        };
        let mut col = Column::new();
        match self.update_state {
            UpdateState::Pending => {
                col = col.push("Checking for updates…");
                col = col.push(Rule::horizontal(1)); //TODO hide if main_view is empty
            }
            UpdateState::UpToDate => {}
            #[cfg(target_os = "macos")] UpdateState::Available(ref new_ver) => {
                col = col.push(Text::new(format!("An update is available ({} → {new_ver})", env!("CARGO_PKG_VERSION"))));
                col = col.push("Please quit this app and run the following command in the Terminal app:");
                col = col.push("brew update && brew upgrade"); //TODO automate
                col = col.push(Rule::horizontal(1));
            }
            UpdateState::Error { ref e, expanded } => {
                let is_network_error = e.is_network_error();
                if expanded {
                    col = col.push(error_view(if is_network_error { "Network error while checking for updates" } else { "Error while checking for updates" }, e, true, self.debug_info_copied.contains(&true)));
                } else {
                    col = col.push(if is_network_error { "Network error while checking for updates" } else { "Error while checking for updates" });
                }
                col = col.push({
                    let mut row = Row::new();
                    if is_network_error {
                        row = row.push(Button::new("Retry").on_press(Message::CheckForUpdates));
                    }
                    row
                        .push(Button::new(if expanded { "Hide Details" } else { "Show Details" }).on_press(Message::ToggleUpdateErrorDetails))
                        .spacing(8)
                });
                col = col.push(Rule::horizontal(1));
            }
        }
        if suppress_scroll { // workaround for https://github.com/iced-rs/iced/issues/2217
            col.push(main_view).spacing(8).padding(8).into()
        } else {
            Scrollable::new(Row::new()
                .push(col.push(main_view).spacing(8).padding(8))
                .push(Space::with_width(Length::Shrink)) // to avoid overlap with the scrollbar
                .spacing(16)
            ).into()
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = Vec::with_capacity(4);
        subscriptions.push(iced::event::listen_with(|event, _, window| if let iced::Event::Window(window::Event::CloseRequested) = event {
            Some(Message::CloseRequested(window))
        } else {
            None
        }));
        if !matches!(self.update_state, UpdateState::Pending) {
            match self.frontend.kind {
                Frontend::Dummy => {}
                Frontend::EverDrive => subscriptions.push(subscription::from_recipe(LoggingSubscription { log: self.log, context: "from EverDrive", inner: everdrive::Subscription { log: self.log } })),
                #[cfg(any(target_os = "linux", target_os = "windows"))] Frontend::BizHawk => if let Some(BizHawkState { port, .. }) = self.frontend.bizhawk {
                    subscriptions.push(subscription::from_recipe(LoggingSubscription { log: self.log, context: "from BizHawk", inner: subscriptions::Connection { port, frontend: self.frontend.kind, log: self.log, connection_id: self.frontend_connection_id } }));
                },
                #[cfg(not(any(target_os = "linux", target_os = "windows")))] Frontend::BizHawk => unreachable!("no BizHawk support on this platform"),
                Frontend::Pj64V3 => subscriptions.push(subscription::from_recipe(LoggingSubscription { log: self.log, context: "from Project64", inner: subscriptions::Listener { frontend: self.frontend.kind, log: self.log, connection_id: self.frontend_connection_id } })),
                Frontend::Pj64V4 => subscriptions.push(subscription::from_recipe(LoggingSubscription { log: self.log, context: "from Project64", inner: subscriptions::Connection { port: frontend::PORT, frontend: self.frontend.kind, log: self.log, connection_id: self.frontend_connection_id } })), //TODO allow Project64 to specify port via command-line arg
            }
            if !matches!(self.server_connection, SessionState::Error { .. } | SessionState::Closed { .. }) {
                subscriptions.push(subscription::from_recipe(LoggingSubscription { log: self.log, context: "from server", inner: subscriptions::Client { log: self.log, websocket_url: self.websocket_url.clone() } }));
            }
            if let SessionState::Lobby { view: LobbyView::Login { provider, no_midos_house_account: false }, .. } = self.server_connection {
                subscriptions.push(subscription::from_recipe(LoggingSubscription { log: self.log, context: "from login handler", inner: login::Subscription(provider) }));
            }
        }
        Subscription::batch(subscriptions)
    }
}

fn error_view<'a>(context: impl Into<Cow<'a, str>>, e: &impl ToString, update: bool, debug_info_copied: bool) -> Column<'a, Message> {
    Column::new()
        .push(Text::new("Error").size(24))
        .push(Text::new(context.into()))
        .push(Text::new(e.to_string()))
        .push(Row::new()
            .push(Button::new("Copy debug info").on_press(Message::CopyDebugInfo(update)))
            .push(if debug_info_copied { "Copied!" } else { "for pasting into Discord" })
            .align_y(iced::Alignment::Center)
            .spacing(8)
        )
        .push(Text::new("Support").size(24))
        .push("This is a bug in Mido's House Multiworld. Please report it:")
        .push(Row::new()
            .push("• ")
            .push(Button::new("Open a GitHub issue").on_press(Message::NewIssue(update)))
            .align_y(iced::Alignment::Center)
            .spacing(8)
        )
        .push("• Or post in #setup-support on the OoT Randomizer Discord. Please ping @fenhl in your message.")
        .push(Row::new()
            .push(Button::new("invite link").on_press(Message::DiscordInvite))
            .push(Button::new("direct channel link").on_press(Message::DiscordChannel))
            .align_y(iced::Alignment::Center)
            .spacing(8)
        )
        .push("• Or post in #general on the OoTR MW Tournament Discord.")
        .spacing(8)
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
    iced::application(State::title, State::update, State::view)
        .subscription(State::subscription)
        .window(window::Settings {
            size: Size { width: 360.0, height: 360.0 },
            exit_on_close_request: false,
            icon,
            ..window::Settings::default()
        })
        .theme(|_| wheel::gui::theme())
        .run_with(|| (
            State::new(icon_error, Config::blocking_load(), PersistentState::blocking_load(), frontend),
            cmd(future::ok(Message::CheckForUpdates)),
        ))
}
