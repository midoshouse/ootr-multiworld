#![deny(rust_2018_idioms, unused, unused_crate_dependencies, unused_import_braces, unused_lifetimes, unused_qualifications, warnings)]
#![forbid(unsafe_code)]

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use {
    std::{
        collections::BTreeSet,
        future::Future,
        num::NonZeroU8,
        sync::Arc,
    },
    async_proto::Protocol as _,
    iced::{
        Command,
        Settings,
        Subscription,
        pure::{
            Application,
            Element,
            widget::*,
        },
        window,
    },
    itertools::Itertools as _,
    tokio::{
        net::tcp::OwnedWriteHalf,
        sync::Mutex,
    },
    multiworld::{
        LobbyClientMessage,
        Player,
        RoomClientMessage,
        ServerMessage,
        format_room_state,
    },
};

mod subscriptions;

const MW_PJ64_PROTO_VERSION: u8 = 0; //TODO sync with JS code

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error(transparent)] Client(#[from] multiworld::ClientError),
    #[error(transparent)] Io(#[from] tokio::io::Error),
    #[error(transparent)] Read(#[from] async_proto::ReadError),
    #[error(transparent)] Write(#[from] async_proto::WriteError),
    #[error("server error: {0}")]
    Server(String),
    #[error("protocol version mismatch: Project64 script is version {0} but we're version {}", MW_PJ64_PROTO_VERSION)]
    VersionMismatch(u8),
}

#[derive(Debug, Clone)]
enum Message {
    CommandError(Arc<Error>),
    JoinRoom,
    Nop,
    Pj64Connected(Arc<Mutex<OwnedWriteHalf>>),
    Pj64SubscriptionError(Arc<Error>),
    Plugin(subscriptions::ClientMessage),
    Rooms(Arc<Mutex<OwnedWriteHalf>>, BTreeSet<String>),
    Server(ServerMessage),
    ServerSubscriptionError(Arc<Error>),
    SetCreateNewRoom(bool),
    SetExistingRoomSelection(String),
    SetNewRoomName(String),
    SetPassword(String),
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
    Error(Arc<Error>),
    Init,
    Lobby {
        rooms: BTreeSet<String>,
        create_new_room: bool,
        existing_room_selection: Option<String>,
        new_room_name: String,
        password: String,
    },
    Room {
        players: Vec<Player>,
        num_unassigned_clients: u8,
    },
}

struct State {
    command_error: Option<Arc<Error>>,
    pj64_subscription_error: Option<Arc<Error>>,
    pj64_writer: Option<Arc<Mutex<OwnedWriteHalf>>>,
    server_connection: ServerConnectionState,
    server_writer: Option<Arc<Mutex<OwnedWriteHalf>>>,
    player_id: Option<NonZeroU8>,
    player_name: Option<[u8; 8]>,
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
            player_id: None,
            player_name: None,
        }, Command::none())
    }

        fn title(&self) -> String { format!("OoTR Multiworld for Project64") }

    fn update(&mut self, msg: Message) -> Command<Message> {
        match msg {
            Message::CommandError(e) => { self.command_error.get_or_insert(e); }
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
            Message::Nop => {}
            Message::Pj64Connected(writer) => self.pj64_writer = Some(writer),
            Message::Pj64SubscriptionError(e) => { self.pj64_subscription_error.get_or_insert(e); }
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
            Message::Rooms(writer, rooms) => {
                self.server_writer = Some(writer);
                self.server_connection = ServerConnectionState::Lobby {
                    create_new_room: rooms.is_empty(),
                    existing_room_selection: None,
                    new_room_name: String::default(),
                    password: String::default(),
                    rooms,
                };
            }
            Message::Server(ServerMessage::EnterRoom { players, num_unassigned_clients }) => {
                self.server_connection = ServerConnectionState::Room { players: players.clone(), num_unassigned_clients };
                let server_writer = self.server_writer.clone().expect("join room button only appears when connected to server");
                let pj64_writer = self.pj64_writer.clone().expect("join room button only appears when connected to server");
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
                        if player.name != Player::DEFAULT_NAME {
                            subscriptions::ServerMessage::PlayerName(player.world, player.name).write(&mut *pj64_writer.lock().await).await?;
                        }
                    }
                    Ok(Message::Nop)
                })
            }
            Message::Server(ServerMessage::Error(e)) => if !matches!(self.server_connection, ServerConnectionState::Error(_)) {
                self.server_connection = ServerConnectionState::Error(Arc::new(Error::Server(e)));
            },
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
                let writer = self.pj64_writer.clone().expect("join room button only appears when connected to server");
                return cmd(async move {
                    subscriptions::ServerMessage::PlayerName(world, name).write(&mut *writer.lock().await).await?;
                    Ok(Message::Nop)
                })
            },
            Message::Server(ServerMessage::ItemQueue(queue)) => {
                let writer = self.pj64_writer.clone().expect("PJ64 not connected");
                return cmd(async move {
                    subscriptions::ServerMessage::ItemQueue(queue).write(&mut *writer.lock().await).await?;
                    Ok(Message::Nop)
                })
            }
            Message::Server(ServerMessage::GetItem(item)) => {
                let writer = self.pj64_writer.clone().expect("PJ64 not connected");
                return cmd(async move {
                    subscriptions::ServerMessage::GetItem(item).write(&mut *writer.lock().await).await?;
                    Ok(Message::Nop)
                })
            }
            Message::ServerSubscriptionError(e) => if !matches!(self.server_connection, ServerConnectionState::Error(_)) {
                self.server_connection = ServerConnectionState::Error(e);
            },
            Message::SetCreateNewRoom(new_val) => if let ServerConnectionState::Lobby { ref mut create_new_room, .. } = self.server_connection { *create_new_room = new_val },
            Message::SetExistingRoomSelection(name) => if let ServerConnectionState::Lobby { ref mut existing_room_selection, .. } = self.server_connection { *existing_room_selection = Some(name) },
            Message::SetNewRoomName(name) => if let ServerConnectionState::Lobby { ref mut new_room_name, .. } = self.server_connection { *new_room_name = name },
            Message::SetPassword(new_password) => if let ServerConnectionState::Lobby { ref mut password, .. } = self.server_connection { *password = new_password },
        }
        Command::none()
    }

    fn view(&self) -> Element<'_, Message> {
        if let Some(ref e) = self.command_error {
            Column::new()
                .push(Text::new("An error occurred:"))
                .push(Text::new(e.to_string()))
                .push(Text::new(format!("Please report this error to Fenhl. Debug info: {e:?}")))
                .spacing(8)
                .padding(8)
                .into()
        } else if let Some(ref e) = self.pj64_subscription_error {
            Column::new()
                .push(Text::new("An error occurred during communication with Project64:"))
                .push(Text::new(e.to_string()))
                .push(Text::new(format!("Please report this error to Fenhl. Debug info: {e:?}")))
                .spacing(8)
                .padding(8)
                .into()
        } else if self.pj64_writer.is_none() {
            Column::new()
                .push(Text::new("Waiting for Project64…\n\n1. In Project64's Debugger menu, select Scripts\n2. In the Scripts window, select ootrmw.js and click Run\n3. Wait until the Output area says “Connected to multiworld app”. (This should take less than 5 seconds.) You can then close the Scripts window."))
                .spacing(8)
                .padding(8)
                .into()
        } else {
            match self.server_connection {
                ServerConnectionState::Error(ref e) => Column::new()
                    .push(Text::new("An error occurred during communication with the server:"))
                    .push(Text::new(e.to_string()))
                    .push(Text::new(format!("Please report this error to Fenhl. Debug info: {e:?}")))
                    .spacing(8)
                    .padding(8)
                    .into(),
                ServerConnectionState::Init => Column::new()
                    .push(Text::new("Connecting to server…"))
                    .spacing(8)
                    .padding(8)
                    .into(),
                ServerConnectionState::Lobby { ref rooms, create_new_room, ref existing_room_selection, ref new_room_name, ref password } => Column::new()
                    .push(Radio::new(false, "Connect to existing room", Some(create_new_room), Message::SetCreateNewRoom))
                    .push(Radio::new(true, "Create new room", Some(create_new_room), Message::SetCreateNewRoom))
                    .push(if create_new_room {
                        Element::from(TextInput::new("Room name", new_room_name, Message::SetNewRoomName).on_submit(Message::JoinRoom).padding(5))
                    } else {
                        if rooms.is_empty() {
                            Text::new("(no rooms currently open)").into()
                        } else {
                            PickList::new(rooms.iter().cloned().collect_vec(), existing_room_selection.clone(), Message::SetExistingRoomSelection).into()
                        }
                    })
                    .push(TextInput::new("Password", password, Message::SetPassword).password().on_submit(Message::JoinRoom).padding(5))
                    .push({
                        let mut btn = Button::new(Text::new("Connect"));
                        if if create_new_room { !new_room_name.is_empty() } else { existing_room_selection.is_some() } && !password.is_empty() { btn = btn.on_press(Message::JoinRoom) }
                        btn
                    })
                    .spacing(8)
                    .padding(8)
                    .into(),
                ServerConnectionState::Room { ref players, num_unassigned_clients, .. } => Column::new()
                    .push(Text::new(format_room_state(players, num_unassigned_clients, self.player_id)))
                    .spacing(8)
                    .padding(8)
                    .into(),
            }
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            Subscription::from_recipe(subscriptions::Pj64Listener),
            Subscription::from_recipe(subscriptions::Client),
        ])
    }
}

#[wheel::main]
fn main() -> iced::Result {
    State::run(Settings {
        window: window::Settings {
            size: (256, 256),
            ..window::Settings::default()
        },
        ..Settings::default()
    })
}
