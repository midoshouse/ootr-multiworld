#![deny(rust_2018_idioms, unused, unused_crate_dependencies, unused_import_braces, unused_lifetimes, unused_qualifications, warnings)]
#![forbid(unsafe_code)]

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use {
    std::{
        collections::BTreeSet,
        env,
        future::Future,
        num::NonZeroU8,
        sync::Arc,
        time::Duration,
    },
    async_proto::Protocol as _,
    chrono::prelude::*,
    dark_light::Mode::*,
    directories::ProjectDirs,
    futures::future,
    iced::{
        Command,
        Settings,
        Subscription,
        pure::{
            Application,
            Element,
            widget::*,
        },
        window::{
            self,
            Icon,
        },
    },
    itertools::Itertools as _,
    semver::Version,
    tokio::{
        fs,
        io,
        net::tcp::OwnedWriteHalf,
        sync::Mutex,
        time::{
            Instant,
            sleep_until,
        },
    },
    multiworld::{
        Filename,
        IsNetworkError,
        LobbyClientMessage,
        Player,
        RoomClientMessage,
        ServerMessage,
        format_room_state,
        github::Repo,
        style::Style,
    },
};

mod subscriptions;

const MW_PJ64_PROTO_VERSION: u8 = 1; //TODO sync with JS code

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error(transparent)] Client(#[from] multiworld::ClientError),
    #[error(transparent)] Elapsed(#[from] tokio::time::error::Elapsed),
    #[error(transparent)] Io(#[from] io::Error),
    #[error(transparent)] Read(#[from] async_proto::ReadError),
    #[error(transparent)] Reqwest(#[from] reqwest::Error),
    #[error(transparent)] Semver(#[from] semver::Error),
    #[error(transparent)] Write(#[from] async_proto::WriteError),
    #[error("user folder not found")]
    MissingHomeDir,
    #[error("server error: {0}")]
    Server(String),
    #[error("protocol version mismatch: Project64 script is version {0} but we're version {}", MW_PJ64_PROTO_VERSION)]
    VersionMismatch(u8),
}

impl IsNetworkError for Error {
    fn is_network_error(&self) -> bool {
        match self {
            Self::Client(e) => e.is_network_error(),
            Self::Elapsed(_) => true,
            Self::Io(e) => e.is_network_error(),
            Self::Read(e) => e.is_network_error(),
            Self::Write(e) => e.is_network_error(),
            _ => false,
        }
    }
}

#[derive(Debug, Clone)]
enum Message {
    CommandError(Arc<Error>),
    DismissWrongPassword,
    Exit,
    JoinRoom,
    Kick(NonZeroU8),
    Nop,
    Pj64Connected(Arc<Mutex<OwnedWriteHalf>>),
    Pj64SubscriptionError(Arc<Error>),
    Plugin(subscriptions::ClientMessage),
    ReconnectToLobby,
    ReconnectToRoom(String, String),
    Rooms(Arc<Mutex<OwnedWriteHalf>>, BTreeSet<String>),
    Server(ServerMessage),
    ServerSubscriptionError(Arc<Error>),
    SetCreateNewRoom(bool),
    SetExistingRoomSelection(String),
    SetNewRoomName(String),
    SetPassword(String),
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

enum ServerConnectionState {
    Error {
        e: Arc<Error>,
        auto_retry: bool,
    },
    Init,
    InitAutoRejoin {
        room_name: String,
        room_password: String,
    },
    Lobby {
        rooms: BTreeSet<String>,
        create_new_room: bool,
        existing_room_selection: Option<String>,
        new_room_name: String,
        password: String,
        wrong_password: bool,
    },
    Room {
        room_name: String,
        room_password: String,
        players: Vec<Player>,
        num_unassigned_clients: u8,
        item_queue: Vec<u16>,
    },
    Closed,
}

struct State {
    command_error: Option<Arc<Error>>,
    pj64_subscription_error: Option<Arc<Error>>,
    pj64_writer: Option<Arc<Mutex<OwnedWriteHalf>>>,
    server_connection: ServerConnectionState,
    server_writer: Option<Arc<Mutex<OwnedWriteHalf>>>,
    retry: Instant,
    wait_time: Duration,
    player_id: Option<NonZeroU8>,
    player_name: Option<Filename>,
    updates_checked: bool,
    should_exit: bool,
}

impl Application for State {
    type Executor = iced::executor::Default;
    type Message = Message;
    type Flags = ();

    fn new((): ()) -> (Self, Command<Message>) {
        (Self {
            command_error: None,
            pj64_subscription_error: None,
            pj64_writer: None,
            server_connection: ServerConnectionState::Init,
            server_writer: None,
            retry: Instant::now(),
            wait_time: Duration::from_secs(1),
            player_id: None,
            player_name: None,
            updates_checked: false,
            should_exit: false,
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
                    let project_dirs = ProjectDirs::from("net", "Fenhl", "OoTR Multiworld").ok_or(Error::MissingHomeDir)?;
                    let cache_dir = project_dirs.cache_dir();
                    fs::create_dir_all(cache_dir).await?;
                    let updater_path = cache_dir.join("updater.exe");
                    #[cfg(target_arch = "x86_64")] let updater_data = include_bytes!("../../../target/release/multiworld-updater.exe");
                    fs::write(&updater_path, updater_data).await?;
                    let _ = std::process::Command::new(updater_path).arg("pj64").arg(env::current_exe()?).spawn()?;
                    return Ok(Message::Exit)
                }
            }
            Ok(Message::UpToDate)
        }))
    }

    fn background_color(&self) -> iced::Color {
        match dark_light::detect() { //TODO automatically update on system theme change
            Dark => iced::Color::BLACK,
            Light => iced::Color::WHITE,
        }
    }

    fn should_exit(&self) -> bool { self.should_exit }

    fn title(&self) -> String { format!("Mido's House Multiworld for Project64") }

    fn update(&mut self, msg: Message) -> Command<Message> {
        match msg {
            Message::CommandError(e) => { self.command_error.get_or_insert(e); }
            Message::DismissWrongPassword => if let ServerConnectionState::Lobby { ref mut wrong_password, .. } = self.server_connection {
                *wrong_password = false;
            },
            Message::Exit => self.should_exit = true,
            Message::JoinRoom => if let ServerConnectionState::Lobby { create_new_room, ref existing_room_selection, ref new_room_name, ref password, .. } = self.server_connection {
                if !password.is_empty() {
                    let existing_room_selection = existing_room_selection.clone();
                    let new_room_name = new_room_name.clone();
                    let password = password.clone();
                    let writer = self.server_writer.clone().expect("join room button only appears when connected to server");
                    return cmd(async move {
                        if create_new_room {
                            if !new_room_name.is_empty() {
                                LobbyClientMessage::CreateRoom { name: new_room_name, password }.write(&mut *writer.lock().await).await?;
                            }
                        } else {
                            if let Some(name) = existing_room_selection {
                                LobbyClientMessage::JoinRoom { name, password }.write(&mut *writer.lock().await).await?;
                            }
                        }
                        Ok(Message::Nop)
                    })
                }
            }
            Message::Kick(player_id) => if let Some(writer) = self.server_writer.clone() {
                return cmd(async move {
                    RoomClientMessage::KickPlayer(player_id).write(&mut *writer.lock().await).await?;
                    Ok(Message::Nop)
                })
            },
            Message::Nop => {}
            Message::Pj64Connected(writer) => {
                self.pj64_writer = Some(Arc::clone(&writer));
                if let ServerConnectionState::Room { ref players, ref item_queue, .. } = self.server_connection {
                    let players = players.clone();
                    let item_queue = item_queue.clone();
                    return cmd(async move {
                        let mut writer = writer.lock().await;
                        for player in players {
                            if player.name != Filename::default() {
                                subscriptions::ServerMessage::PlayerName(player.world, player.name).write(&mut *writer).await?;
                            }
                        }
                        if !item_queue.is_empty() {
                            subscriptions::ServerMessage::ItemQueue(item_queue).write(&mut *writer).await?;
                        }
                        Ok(Message::Nop)
                    })
                }
            }
            Message::Pj64SubscriptionError(e) => {
                if let Error::Read(async_proto::ReadError::Io(ref e)) = *e {
                    if e.kind() == io::ErrorKind::ConnectionReset {
                        self.pj64_writer = None;
                        return Command::none()
                    }
                }
                self.pj64_subscription_error.get_or_insert(e);
            }
            Message::Plugin(subscriptions::ClientMessage::PlayerId(new_player_id)) => {
                let new_player_name = self.player_id.replace(new_player_id).is_none().then_some(self.player_name).flatten();
                if let Some(ref writer) = self.server_writer {
                    if let ServerConnectionState::Room { .. } = self.server_connection {
                        let writer = writer.clone();
                        return cmd(async move {
                            RoomClientMessage::PlayerId(new_player_id).write(&mut *writer.lock().await).await?;
                            if let Some(new_player_name) = new_player_name {
                                RoomClientMessage::PlayerName(new_player_name).write(&mut *writer.lock().await).await?;
                            }
                            Ok(Message::Nop)
                        })
                    }
                }
            }
            Message::Plugin(subscriptions::ClientMessage::PlayerName(new_player_name)) => {
                self.player_name = Some(new_player_name);
                if self.player_id.is_some() {
                    if let Some(ref writer) = self.server_writer {
                        if let ServerConnectionState::Room { .. } = self.server_connection {
                            let writer = writer.clone();
                            return cmd(async move {
                                RoomClientMessage::PlayerName(new_player_name).write(&mut *writer.lock().await).await?;
                                Ok(Message::Nop)
                            })
                        }
                    }
                }
            }
            Message::Plugin(subscriptions::ClientMessage::SendItem { key, kind, target_world }) => {
                let writer = self.server_writer.clone().expect("trying to send an item but not connected to server");
                return cmd(async move {
                    RoomClientMessage::SendItem { key, kind, target_world }.write(&mut *writer.lock().await).await?;
                    Ok(Message::Nop)
                })
            }
            Message::ReconnectToLobby => self.server_connection = ServerConnectionState::Init,
            Message::ReconnectToRoom(room_name, room_password) => self.server_connection = ServerConnectionState::InitAutoRejoin { room_name, room_password },
            Message::Rooms(writer, rooms) => {
                self.server_writer = Some(writer);
                let mut room_still_exists = false;
                self.server_connection = if let ServerConnectionState::InitAutoRejoin { ref room_name, ref room_password } = self.server_connection {
                    room_still_exists = rooms.contains(room_name);
                    ServerConnectionState::Lobby {
                        create_new_room: !room_still_exists,
                        existing_room_selection: room_still_exists.then(|| room_name.clone()),
                        new_room_name: room_name.clone(),
                        password: room_password.clone(),
                        wrong_password: false,
                        rooms,
                    }
                } else {
                    ServerConnectionState::Lobby {
                        create_new_room: rooms.is_empty(),
                        existing_room_selection: None,
                        new_room_name: String::default(),
                        password: String::default(),
                        wrong_password: false,
                        rooms,
                    }
                };
                if room_still_exists {
                    return cmd(future::ok(Message::JoinRoom))
                }
            }
            Message::Server(ServerMessage::OtherError(e)) => if !matches!(self.server_connection, ServerConnectionState::Error { .. }) {
                self.server_connection = ServerConnectionState::Error {
                    e: Arc::new(Error::Server(e)),
                    auto_retry: false,
                };
            },
            Message::Server(ServerMessage::NewRoom(name)) => if let ServerConnectionState::Lobby { ref mut rooms, .. } = self.server_connection { rooms.insert(name); },
            Message::Server(ServerMessage::EnterRoom { players, num_unassigned_clients }) => {
                let (room_name, room_password) = match self.server_connection {
                    ServerConnectionState::Lobby { create_new_room: false, ref existing_room_selection, ref password, .. } => (existing_room_selection.clone().unwrap_or_default(), password.clone()),
                    ServerConnectionState::Lobby { create_new_room: true, ref new_room_name, ref password, .. } => (new_room_name.clone(), password.clone()),
                    _ => <_>::default(),
                };
                self.server_connection = ServerConnectionState::Room { players: players.clone(), item_queue: Vec::default(), room_name, room_password, num_unassigned_clients };
                let server_writer = self.server_writer.clone().expect("join room button only appears when connected to server");
                let pj64_writer = self.pj64_writer.clone().expect("join room button only appears when connected to PJ64");
                let player_id = self.player_id;
                let player_name = self.player_name;
                return cmd(async move {
                    if let Some(player_id) = player_id {
                        RoomClientMessage::PlayerId(player_id).write(&mut *server_writer.lock().await).await?;
                        if let Some(player_name) = player_name {
                            RoomClientMessage::PlayerName(player_name).write(&mut *server_writer.lock().await).await?;
                        }
                    }
                    for player in players {
                        if player.name != Filename::default() {
                            subscriptions::ServerMessage::PlayerName(player.world, player.name).write(&mut *pj64_writer.lock().await).await?;
                        }
                    }
                    Ok(Message::Nop)
                })
            }
            Message::Server(ServerMessage::PlayerId(world)) => if let ServerConnectionState::Room { ref mut players, ref mut num_unassigned_clients, .. } = self.server_connection {
                if let Err(idx) = players.binary_search_by_key(&world, |p| p.world) {
                    players.insert(idx, Player::new(world));
                    *num_unassigned_clients -= 1;
                }
            },
            Message::Server(ServerMessage::ResetPlayerId(world)) => if let ServerConnectionState::Room { ref mut players, ref mut num_unassigned_clients, .. } = self.server_connection {
                if let Ok(idx) = players.binary_search_by_key(&world, |p| p.world) {
                    players.remove(idx);
                    *num_unassigned_clients += 1;
                }
            },
            Message::Server(ServerMessage::ClientConnected) => if let ServerConnectionState::Room { ref mut num_unassigned_clients, .. } = self.server_connection { *num_unassigned_clients += 1 },
            Message::Server(ServerMessage::PlayerDisconnected(world)) => if let ServerConnectionState::Room { ref mut players, .. } = self.server_connection {
                if let Ok(idx) = players.binary_search_by_key(&world, |p| p.world) {
                    players.remove(idx);
                }
            },
            Message::Server(ServerMessage::UnregisteredClientDisconnected) => if let ServerConnectionState::Room { ref mut num_unassigned_clients, .. } = self.server_connection { *num_unassigned_clients -= 1 },
            Message::Server(ServerMessage::PlayerName(world, name)) => if let ServerConnectionState::Room { ref mut players, .. } = self.server_connection {
                if let Ok(idx) = players.binary_search_by_key(&world, |p| p.world) {
                    players[idx].name = name;
                }
                if let Some(writer) = self.pj64_writer.clone() {
                    return cmd(async move {
                        subscriptions::ServerMessage::PlayerName(world, name).write(&mut *writer.lock().await).await?;
                        Ok(Message::Nop)
                    })
                }
            },
            Message::Server(ServerMessage::ItemQueue(queue)) => if let ServerConnectionState::Room { ref mut item_queue, .. } = self.server_connection {
                *item_queue = queue.clone();
                if let Some(writer) = self.pj64_writer.clone() {
                    return cmd(async move {
                        subscriptions::ServerMessage::ItemQueue(queue).write(&mut *writer.lock().await).await?;
                        Ok(Message::Nop)
                    })
                }
            }
            Message::Server(ServerMessage::GetItem(item)) => if let ServerConnectionState::Room { ref mut item_queue, .. } = self.server_connection {
                item_queue.push(item);
                if let Some(writer) = self.pj64_writer.clone() {
                    return cmd(async move {
                        subscriptions::ServerMessage::GetItem(item).write(&mut *writer.lock().await).await?;
                        Ok(Message::Nop)
                    })
                }
            }
            Message::Server(ServerMessage::AdminLoginSuccess { .. }) => unreachable!(),
            Message::Server(ServerMessage::WrongPassword) => if let ServerConnectionState::Lobby { ref mut password, ref mut wrong_password, .. } = self.server_connection {
                *wrong_password = true;
                password.clear();
            },
            Message::Server(ServerMessage::Goodbye) => if !matches!(self.server_connection, ServerConnectionState::Error { .. }) {
                self.server_connection = ServerConnectionState::Closed;
            },
            Message::Server(ServerMessage::Ping) => {}
            Message::ServerSubscriptionError(e) => if !matches!(self.server_connection, ServerConnectionState::Error { .. }) {
                if e.is_network_error() {
                    if self.retry.elapsed() >= Duration::from_secs(60 * 60 * 24) {
                        self.wait_time = Duration::from_secs(1); // reset wait time after no error for a day
                    } else {
                        self.wait_time *= 2; // exponential backoff
                    }
                    self.retry = Instant::now() + self.wait_time;
                    let retry = self.retry;
                    let reconnect_msg = if let ServerConnectionState::Room { ref room_name, ref room_password, .. } = self.server_connection {
                        Message::ReconnectToRoom(room_name.clone(), room_password.clone())
                    } else {
                        Message::ReconnectToLobby
                    };
                    self.server_connection = ServerConnectionState::Error {
                        auto_retry: true,
                        e,
                    };
                    return cmd(async move {
                        sleep_until(retry).await;
                        Ok(reconnect_msg)
                    })
                } else {
                    self.server_connection = ServerConnectionState::Error {
                        auto_retry: false,
                        e,
                    };
                }
            },
            Message::SetCreateNewRoom(new_val) => if let ServerConnectionState::Lobby { ref mut create_new_room, .. } = self.server_connection { *create_new_room = new_val },
            Message::SetExistingRoomSelection(name) => if let ServerConnectionState::Lobby { ref mut existing_room_selection, .. } = self.server_connection { *existing_room_selection = Some(name) },
            Message::SetNewRoomName(name) => if let ServerConnectionState::Lobby { ref mut new_room_name, .. } = self.server_connection { *new_room_name = name },
            Message::SetPassword(new_password) => if let ServerConnectionState::Lobby { ref mut password, .. } = self.server_connection { *password = new_password },
            Message::UpToDate => self.updates_checked = true,
        }
        Command::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let system_theme = dark_light::detect(); //TODO automatically update on system theme change
        let text_color = match system_theme {
            Dark => iced::Color::WHITE,
            Light => iced::Color::BLACK,
        };
        if let Some(ref e) = self.command_error {
            Column::new()
                .push(Text::new("An error occurred:").color(text_color))
                .push(Text::new(e.to_string()).color(text_color))
                .push(Text::new(format!("Please report this error to Fenhl. Debug info: {e:?}")).color(text_color))
                .spacing(8)
                .padding(8)
                .into()
        } else if let Some(ref e) = self.pj64_subscription_error {
            Column::new()
                .push(Text::new("An error occurred during communication with Project64:").color(text_color))
                .push(Text::new(e.to_string()).color(text_color))
                .push(Text::new(format!("Please report this error to Fenhl. Debug info: {e:?}")).color(text_color))
                .spacing(8)
                .padding(8)
                .into()
        } else if !self.updates_checked {
            Column::new()
                .push(Text::new("Checking for updates…").color(text_color))
                .spacing(8)
                .padding(8)
                .into()
        } else if self.pj64_writer.is_none() {
            Column::new()
                .push(Text::new("Waiting for Project64…\n\n1. In Project64's Debugger menu, select Scripts\n2. In the Scripts window, select ootrmw.js and click Run\n3. Wait until the Output area says “Connected to multiworld app”. (This should take less than 5 seconds.) You can then close the Scripts window.").color(text_color))
                .spacing(8)
                .padding(8)
                .into()
        } else {
            match self.server_connection {
                ServerConnectionState::Error { auto_retry: false, ref e } => Column::new()
                    .push(Text::new("An error occurred during communication with the server:").color(text_color))
                    .push(Text::new(e.to_string()).color(text_color))
                    .push(Text::new(format!("Please report this error to Fenhl. Debug info: {e:?}")).color(text_color))
                    .spacing(8)
                    .padding(8)
                    .into(),
                ServerConnectionState::Error { auto_retry: true, ref e } => Column::new()
                    .push(Text::new("A network error occurred:").color(text_color))
                    .push(Text::new(e.to_string()).color(text_color))
                    .push(Text::new(if let Ok(retry) = chrono::Duration::from_std(self.retry.duration_since(Instant::now())) {
                        format!("Reconnecting at {}", (Local::now() + retry).format("%H:%M:%S"))
                    } else {
                        format!("Reconnecting…")
                    }).color(text_color)) //TODO live countdown
                    .spacing(8)
                    .padding(8)
                    .into(),
                ServerConnectionState::Init => Column::new()
                    .push(Text::new("Connecting to server…").color(text_color))
                    .spacing(8)
                    .padding(8)
                    .into(),
                ServerConnectionState::InitAutoRejoin { .. } => Column::new()
                    .push(Text::new("Reconnecting to room…").color(text_color))
                    .spacing(8)
                    .padding(8)
                    .into(),
                ServerConnectionState::Lobby { wrong_password: true, .. } => Column::new()
                    .push(Text::new("wrong password").color(text_color))
                    .push(Button::new(Text::new("OK").color(text_color)).on_press(Message::DismissWrongPassword).style(Style(system_theme)))
                    .spacing(8)
                    .padding(8)
                    .into(),
                ServerConnectionState::Lobby { wrong_password: false, ref rooms, create_new_room, ref existing_room_selection, ref new_room_name, ref password } => Column::new()
                    .push(Radio::new(false, "Connect to existing room", Some(create_new_room), Message::SetCreateNewRoom).style(Style(system_theme)))
                    .push(Radio::new(true, "Create new room", Some(create_new_room), Message::SetCreateNewRoom).style(Style(system_theme)))
                    .push(if create_new_room {
                        Element::from(TextInput::new("Room name", new_room_name, Message::SetNewRoomName).on_submit(Message::JoinRoom).padding(5).style(Style(system_theme)))
                    } else {
                        if rooms.is_empty() {
                            Text::new("(no rooms currently open)").color(text_color).into()
                        } else {
                            PickList::new(rooms.iter().cloned().collect_vec(), existing_room_selection.clone(), Message::SetExistingRoomSelection).style(Style(system_theme)).into()
                        }
                    })
                    .push(TextInput::new("Password", password, Message::SetPassword).password().on_submit(Message::JoinRoom).padding(5).style(Style(system_theme)))
                    .push({
                        let mut btn = Button::new(Text::new("Connect").color(text_color)).style(Style(system_theme));
                        if if create_new_room { !new_room_name.is_empty() } else { existing_room_selection.is_some() } && !password.is_empty() { btn = btn.on_press(Message::JoinRoom) }
                        btn
                    })
                    .spacing(8)
                    .padding(8)
                    .into(),
                ServerConnectionState::Room { ref players, num_unassigned_clients, .. } => {
                    let mut col = Column::new();
                    let (players, other) = format_room_state(players, num_unassigned_clients, self.player_id);
                    for (player_idx, player) in players.into_iter().enumerate() {
                        let player_id = NonZeroU8::new(u8::try_from(player_idx + 1).expect("too many players")).expect("iterator index + 1 was 0");
                        let mut row = Row::new();
                        row = row.push(Text::new(player).color(text_color));
                        if self.player_id.map_or(true, |my_id| my_id != player_id) {
                            row = row.push(Button::new(Text::new("Kick").color(text_color)).on_press(Message::Kick(player_id)).style(Style(system_theme)));
                        }
                        col = col.push(row);
                    }
                    col
                        .push(Text::new(other).color(text_color))
                        .spacing(8)
                        .padding(8)
                        .into()
                }
                ServerConnectionState::Closed => Column::new()
                    .push(Text::new("You have been disconnected.").color(text_color))
                    .push(Button::new(Text::new("Reconnect").color(text_color)).on_press(Message::ReconnectToLobby).style(Style(system_theme)))
                    .spacing(8)
                    .padding(8)
                    .into(),
            }
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = Vec::with_capacity(2);
        if self.updates_checked {
            subscriptions.push(Subscription::from_recipe(subscriptions::Pj64Listener));
            if !matches!(self.server_connection, ServerConnectionState::Error { .. } | ServerConnectionState::Closed) {
                subscriptions.push(Subscription::from_recipe(subscriptions::Client));
            }
        }
        Subscription::batch(subscriptions)
    }
}

#[derive(Debug, thiserror::Error)]
enum MainError {
    #[error(transparent)] Iced(#[from] iced::Error),
    #[error(transparent)] Icon(#[from] iced::window::icon::Error),
}

#[wheel::main]
fn main() -> Result<(), MainError> {
    let icon = ::image::load_from_memory(include_bytes!("../../../assets/icon.ico")).expect("failed to load embedded DynamicImage").to_rgba8();
    State::run(Settings {
        window: window::Settings {
            size: (256, 256),
            icon: Some(Icon::from_rgba(icon.as_flat_samples().as_slice().to_owned(), icon.width(), icon.height())?),
            ..window::Settings::default()
        },
        ..Settings::default()
    })?;
    Ok(())
}
