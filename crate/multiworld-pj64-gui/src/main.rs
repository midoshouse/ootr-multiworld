#![deny(rust_2018_idioms, unused, unused_crate_dependencies, unused_import_braces, unused_lifetimes, unused_qualifications, warnings)]
#![forbid(unsafe_code)]

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use {
    std::{
        env,
        future::Future,
        num::NonZeroU8,
        process,
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
        Length,
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
        ClientMessage,
        Filename,
        HashIcon,
        IsNetworkError,
        ServerMessage,
        SessionState,
        SessionStateError,
        format_room_state,
        github::Repo,
        style::Style,
    },
};

mod subscriptions;

const MW_PJ64_PROTO_VERSION: u8 = 2; //TODO sync with JS code

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
    CancelRoomDeletion,
    CommandError(Arc<Error>),
    ConfirmRoomDeletion,
    DeleteRoom,
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
    Server(ServerMessage),
    ServerConnected(Arc<Mutex<OwnedWriteHalf>>),
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

struct State {
    command_error: Option<Arc<Error>>,
    pj64_subscription_error: Option<Arc<Error>>,
    pj64_writer: Option<Arc<Mutex<OwnedWriteHalf>>>,
    port: u16,
    server_connection: SessionState<Arc<Error>>,
    server_writer: Option<Arc<Mutex<OwnedWriteHalf>>>,
    retry: Instant,
    wait_time: Duration,
    last_world: Option<NonZeroU8>,
    last_name: Filename,
    last_hash: Option<[HashIcon; 5]>,
    last_save: Option<oottracker::Save>,
    updates_checked: bool,
    should_exit: bool,
}

impl Application for State {
    type Executor = iced::executor::Default;
    type Message = Message;
    type Flags = u16;

    fn new(port: u16) -> (Self, Command<Message>) {
        (Self {
            command_error: None,
            pj64_subscription_error: None,
            pj64_writer: None,
            server_connection: SessionState::Init,
            server_writer: None,
            retry: Instant::now(),
            wait_time: Duration::from_secs(1),
            last_world: None,
            last_name: Filename::default(),
            last_hash: None,
            last_save: None,
            updates_checked: false,
            should_exit: false,
            port,
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
                    let _ = std::process::Command::new(updater_path)
                        .arg("pj64")
                        .arg(env::current_exe()?)
                        .arg(process::id().to_string())
                        .spawn()?;
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
            Message::CancelRoomDeletion => if let SessionState::Room { ref mut confirm_deletion, .. } = self.server_connection {
                *confirm_deletion = false;
            },
            Message::CommandError(e) => { self.command_error.get_or_insert(e); }
            Message::ConfirmRoomDeletion => if let Some(writer) = self.server_writer.clone() {
                return cmd(async move {
                    ClientMessage::DeleteRoom.write(&mut *writer.lock().await).await?;
                    Ok(Message::Nop)
                })
            },
            Message::DeleteRoom => if let SessionState::Room { ref mut confirm_deletion, .. } = self.server_connection {
                *confirm_deletion = true;
            },
            Message::DismissWrongPassword => if let SessionState::Lobby { ref mut wrong_password, .. } = self.server_connection {
                *wrong_password = false;
            },
            Message::Exit => self.should_exit = true,
            Message::JoinRoom => if let SessionState::Lobby { create_new_room, ref existing_room_selection, ref new_room_name, ref password, .. } = self.server_connection {
                if !password.is_empty() {
                    let existing_room_selection = existing_room_selection.clone();
                    let new_room_name = new_room_name.clone();
                    let password = password.clone();
                    let writer = self.server_writer.clone().expect("join room button only appears when connected to server");
                    return cmd(async move {
                        if create_new_room {
                            if !new_room_name.is_empty() {
                                ClientMessage::CreateRoom { name: new_room_name, password }.write(&mut *writer.lock().await).await?;
                            }
                        } else {
                            if let Some(name) = existing_room_selection {
                                ClientMessage::JoinRoom { name, password }.write(&mut *writer.lock().await).await?;
                            }
                        }
                        Ok(Message::Nop)
                    })
                }
            }
            Message::Kick(player_id) => if let Some(writer) = self.server_writer.clone() {
                return cmd(async move {
                    ClientMessage::KickPlayer(player_id).write(&mut *writer.lock().await).await?;
                    Ok(Message::Nop)
                })
            },
            Message::Nop => {}
            Message::Pj64Connected(writer) => {
                self.pj64_writer = Some(Arc::clone(&writer));
                if let SessionState::Room { ref players, ref item_queue, .. } = self.server_connection {
                    let players = players.clone();
                    let item_queue = item_queue.clone();
                    return cmd(async move {
                        let mut writer = writer.lock().await;
                        for player in players {
                            subscriptions::ServerMessage::PlayerName(player.world, if player.name == Filename::default() {
                                Filename::fallback(player.world)
                            } else {
                                player.name
                            }).write(&mut *writer).await?;
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
                            ClientMessage::PlayerId(new_player_id).write(&mut *writer.lock().await).await?;
                            if let Some(new_player_name) = new_player_name {
                                ClientMessage::PlayerName(new_player_name).write(&mut *writer.lock().await).await?;
                            }
                            if let Some(new_file_hash) = new_file_hash {
                                ClientMessage::FileHash(new_file_hash).write(&mut *writer.lock().await).await?;
                            }
                            Ok(Message::Nop)
                        })
                    }
                }
            }
            Message::Plugin(subscriptions::ClientMessage::PlayerName(new_player_name)) => {
                self.last_name = new_player_name;
                if self.last_world.is_some() {
                    if let Some(ref writer) = self.server_writer {
                        if let SessionState::Room { .. } = self.server_connection {
                            let writer = writer.clone();
                            return cmd(async move {
                                ClientMessage::PlayerName(new_player_name).write(&mut *writer.lock().await).await?;
                                Ok(Message::Nop)
                            })
                        }
                    }
                }
            }
            Message::Plugin(subscriptions::ClientMessage::SendItem { key, kind, target_world }) => {
                let writer = self.server_writer.clone().expect("trying to send an item but not connected to server");
                return cmd(async move {
                    ClientMessage::SendItem { key, kind, target_world }.write(&mut *writer.lock().await).await?;
                    Ok(Message::Nop)
                })
            }
            Message::Plugin(subscriptions::ClientMessage::SaveData(save)) => match oottracker::Save::from_save_data(&save) {
                Ok(save) => {
                    self.last_save = Some(save.clone());
                    if let Some(writer) = self.server_writer.clone() {
                        return cmd(async move {
                            ClientMessage::SaveData(save).write(&mut *writer.lock().await).await?; //TODO only send if room is marked as being tracked?
                            Ok(Message::Nop)
                        })
                    }
                }
                Err(e) => if let Some(writer) = self.server_writer.clone() {
                    return cmd(async move {
                        ClientMessage::SaveDataError { debug: format!("{e:?}"), version: multiworld::version() }.write(&mut *writer.lock().await).await?;
                        Ok(Message::Nop)
                    })
                },
            },
            Message::Plugin(subscriptions::ClientMessage::FileHash(new_hash)) => {
                self.last_hash = Some(new_hash);
                if self.last_world.is_some() {
                    if let Some(ref writer) = self.server_writer {
                        if let SessionState::Room { .. } = self.server_connection {
                            let writer = writer.clone();
                            return cmd(async move {
                                ClientMessage::FileHash(new_hash).write(&mut *writer.lock().await).await?;
                                Ok(Message::Nop)
                            })
                        }
                    }
                }
            }
            Message::ReconnectToLobby => self.server_connection = SessionState::Init,
            Message::ReconnectToRoom(room_name, room_password) => self.server_connection = SessionState::InitAutoRejoin { room_name, room_password },
            Message::Server(msg) => {
                let room_still_exists = if let ServerMessage::EnterLobby { ref rooms } = msg {
                    if let SessionState::InitAutoRejoin { ref room_name, .. } = self.server_connection {
                        rooms.contains(room_name)
                    } else {
                        false
                    }
                } else {
                    false
                };
                self.server_connection.apply(msg.clone());
                match msg {
                    ServerMessage::EnterLobby { .. } => if room_still_exists {
                        return cmd(future::ok(Message::JoinRoom))
                    },
                    ServerMessage::EnterRoom { players, .. } => {
                        let server_writer = self.server_writer.clone().expect("join room button only appears when connected to server");
                        let pj64_writer = self.pj64_writer.clone().expect("join room button only appears when connected to PJ64");
                        let player_id = self.last_world;
                        let player_name = self.last_name;
                        let file_hash = self.last_hash;
                        let save = self.last_save.clone();
                        return cmd(async move {
                            if let Some(player_id) = player_id {
                                ClientMessage::PlayerId(player_id).write(&mut *server_writer.lock().await).await?;
                                if player_name != Filename::default() {
                                    ClientMessage::PlayerName(player_name).write(&mut *server_writer.lock().await).await?;
                                }
                                if let Some(hash) = file_hash {
                                    ClientMessage::FileHash(hash).write(&mut *server_writer.lock().await).await?;
                                }
                            }
                            if let Some(save) = save {
                                ClientMessage::SaveData(save).write(&mut *server_writer.lock().await).await?; //TODO only send if room is marked as being tracked?
                            }
                            for player in players {
                                subscriptions::ServerMessage::PlayerName(player.world, if player.name == Filename::default() {
                                    Filename::fallback(player.world)
                                } else {
                                    player.name
                                }).write(&mut *pj64_writer.lock().await).await?;
                            }
                            Ok(Message::Nop)
                        })
                    }
                    ServerMessage::PlayerName(world, name) => if let Some(writer) = self.pj64_writer.clone() {
                        return cmd(async move {
                            subscriptions::ServerMessage::PlayerName(world, name).write(&mut *writer.lock().await).await?;
                            Ok(Message::Nop)
                        })
                    },
                    ServerMessage::ItemQueue(queue) => if let Some(writer) = self.pj64_writer.clone() {
                        return cmd(async move {
                            subscriptions::ServerMessage::ItemQueue(queue).write(&mut *writer.lock().await).await?;
                            Ok(Message::Nop)
                        })
                    },
                    ServerMessage::GetItem(item) => if let Some(writer) = self.pj64_writer.clone() {
                        return cmd(async move {
                            subscriptions::ServerMessage::GetItem(item).write(&mut *writer.lock().await).await?;
                            Ok(Message::Nop)
                        })
                    },
                    _ => {}
                }
            }
            Message::ServerConnected(writer) => self.server_writer = Some(writer),
            Message::ServerSubscriptionError(e) => if !matches!(self.server_connection, SessionState::Error { .. }) {
                if e.is_network_error() {
                    if self.retry.elapsed() >= Duration::from_secs(60 * 60 * 24) {
                        self.wait_time = Duration::from_secs(1); // reset wait time after no error for a day
                    } else {
                        self.wait_time *= 2; // exponential backoff
                    }
                    self.retry = Instant::now() + self.wait_time;
                    let retry = self.retry;
                    let reconnect_msg = if let SessionState::Room { ref room_name, ref room_password, .. } = self.server_connection {
                        Message::ReconnectToRoom(room_name.clone(), room_password.clone())
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
            Message::SetCreateNewRoom(new_val) => if let SessionState::Lobby { ref mut create_new_room, .. } = self.server_connection { *create_new_room = new_val },
            Message::SetExistingRoomSelection(name) => if let SessionState::Lobby { ref mut existing_room_selection, .. } = self.server_connection { *existing_room_selection = Some(name) },
            Message::SetNewRoomName(name) => if let SessionState::Lobby { ref mut new_room_name, .. } = self.server_connection { *new_room_name = name },
            Message::SetPassword(new_password) => if let SessionState::Lobby { ref mut password, .. } = self.server_connection { *password = new_password },
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
                SessionState::Error { auto_retry: false, ref e } => Column::new()
                    .push(Text::new("An error occurred during communication with the server:").color(text_color))
                    .push(Text::new(e.to_string()).color(text_color))
                    .push(Text::new(format!("Please report this error to Fenhl. Debug info: {e:?}")).color(text_color))
                    .spacing(8)
                    .padding(8)
                    .into(),
                SessionState::Error { auto_retry: true, ref e } => Column::new()
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
                SessionState::Init => Column::new()
                    .push(Text::new("Connecting to server…").color(text_color))
                    .spacing(8)
                    .padding(8)
                    .into(),
                SessionState::InitAutoRejoin { .. } => Column::new()
                    .push(Text::new("Reconnecting to room…").color(text_color))
                    .spacing(8)
                    .padding(8)
                    .into(),
                SessionState::Lobby { wrong_password: true, .. } => Column::new()
                    .push(Text::new("wrong password").color(text_color))
                    .push(Button::new(Text::new("OK").color(text_color)).on_press(Message::DismissWrongPassword).style(Style(system_theme)))
                    .spacing(8)
                    .padding(8)
                    .into(),
                SessionState::Lobby { wrong_password: false, ref rooms, create_new_room, ref existing_room_selection, ref new_room_name, ref password, .. } => Column::new()
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
                    .push(Row::new()
                        .push({
                            let mut btn = Button::new(Text::new("Connect").color(text_color)).style(Style(system_theme));
                            if if create_new_room { !new_room_name.is_empty() } else { existing_room_selection.is_some() } && !password.is_empty() { btn = btn.on_press(Message::JoinRoom) }
                            btn
                        })
                        .push(Space::with_width(Length::Fill))
                        .push(Text::new(concat!("v", env!("CARGO_PKG_VERSION"))).color(text_color))
                    )
                    .spacing(8)
                    .padding(8)
                    .into(),
                SessionState::Room { confirm_deletion: true, .. } => Column::new()
                    .push(Text::new("Are you sure you want to delete this room? Items that have already been sent will be lost forever!").color(text_color))
                    .push(Row::new()
                        .push(Button::new(Text::new("Delete").color(text_color)).on_press(Message::ConfirmRoomDeletion).style(Style(system_theme)))
                        .push(Button::new(Text::new("Back").color(text_color)).on_press(Message::CancelRoomDeletion).style(Style(system_theme)))
                        .spacing(8)
                    )
                    .spacing(8)
                    .padding(8)
                    .into(),
                SessionState::Room { confirm_deletion: false, ref players, num_unassigned_clients, .. } => {
                    let mut col = Column::new()
                        .push(Button::new(Text::new("Delete Room").color(text_color)).on_press(Message::DeleteRoom).style(Style(system_theme)));
                    let (players, other) = format_room_state(players, num_unassigned_clients, self.last_world);
                    for (player_id, player) in players.into_iter() {
                        col = col.push(Row::new()
                            .push(Text::new(player).color(text_color))
                            .push(Button::new(Text::new(if self.last_world.map_or(false, |my_id| my_id == player_id) { "Leave" } else { "Kick" }).color(text_color)).on_press(Message::Kick(player_id)).style(Style(system_theme)))
                        );
                    }
                    col
                        .push(Text::new(other).color(text_color))
                        .spacing(8)
                        .padding(8)
                        .into()
                }
                SessionState::Closed => Column::new()
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
            if !matches!(self.server_connection, SessionState::Error { .. } | SessionState::Closed) {
                subscriptions.push(Subscription::from_recipe(subscriptions::Client(self.port)));
            }
        }
        Subscription::batch(subscriptions)
    }
}

#[derive(clap::Parser)]
#[clap(version)]
struct Args {
    #[clap(short, long, default_value_t = multiworld::PORT)]
    port: u16,
}

#[derive(Debug, thiserror::Error)]
enum MainError {
    #[error(transparent)] Iced(#[from] iced::Error),
    #[error(transparent)] Icon(#[from] iced::window::icon::Error),
}

#[wheel::main]
fn main(Args { port }: Args) -> Result<(), MainError> {
    let icon = ::image::load_from_memory(include_bytes!("../../../assets/icon.ico")).expect("failed to load embedded DynamicImage").to_rgba8();
    State::run(Settings {
        window: window::Settings {
            size: (256, 256),
            icon: Some(Icon::from_rgba(icon.as_flat_samples().as_slice().to_owned(), icon.width(), icon.height())?),
            ..window::Settings::default()
        },
        ..Settings::with_flags(port)
    })?;
    Ok(())
}
