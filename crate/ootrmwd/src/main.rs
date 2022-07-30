#![deny(rust_2018_idioms, unused, unused_crate_dependencies, unused_import_braces, unused_lifetimes, unused_qualifications, warnings)]
#![forbid(unsafe_code)]

use {
    std::{
        collections::HashMap,
        convert::{
            Infallible as Never,
            TryFrom as _,
        },
        net::Ipv6Addr,
        pin::Pin,
        sync::Arc,
    },
    async_proto::Protocol as _,
    chrono::prelude::*,
    futures::{
        future::Future,
        stream::{
            Stream,
            StreamExt as _,
        },
    },
    tokio::{
        io,
        net::{
            TcpListener,
            tcp::{
                OwnedReadHalf,
                OwnedWriteHalf,
            },
        },
        select,
        sync::{
            Mutex,
            RwLock,
            broadcast,
            mpsc,
        },
    },
    tokio_stream::wrappers::ReceiverStream,
    multiworld::{
        LobbyClientMessage,
        Player,
        Room,
        RoomClientMessage,
        ServerMessage,
    },
};

#[derive(Debug, thiserror::Error)]
enum SessionError {
    #[error(transparent)] Read(#[from] async_proto::ReadError),
    #[error(transparent)] Write(#[from] async_proto::WriteError),
    #[error("protocol version mismatch: client is version {0} but we're version {}", multiworld::VERSION)]
    VersionMismatch(u8),
}

async fn client_session(rooms_handle: ctrlflow::Handle<Rooms>, socket_id: multiworld::SocketId, mut reader: OwnedReadHalf, writer: Arc<Mutex<OwnedWriteHalf>>) -> Result<(), SessionError> {
    macro_rules! error {
        ($($msg:tt)*) => {{
            let msg = format!($($msg)*);
            ServerMessage::Error(msg).write(&mut *writer.lock().await).await?;
            return Ok(())
        }};
    }

    multiworld::VERSION.write(&mut *writer.lock().await).await?;
    let client_version = u8::read(&mut reader).await?;
    if client_version != multiworld::VERSION { return Err::<(), _>(SessionError::VersionMismatch(client_version)) }
    let (mut room_tx, mut rooms, mut room_stream) = {
        // finish handshake by sending room list (treated as a single packet)
        let mut writer = writer.lock().await;
        let (init, stream) = rooms_handle.stream().await;
        let (tx, rooms) = init.clone();
        u64::try_from(rooms.len()).expect("too many rooms").write(&mut *writer).await?;
        for room_name in rooms.keys() {
            room_name.write(&mut *writer).await?;
        }
        (tx, rooms, stream)
    };
    let room = {
        let mut read = LobbyClientMessage::read(&mut reader);
        loop {
            select! {
                new_room = room_stream.recv() => match new_room {
                    Ok(NewRoom { name, room }) => {
                        ServerMessage::NewRoom(name.clone()).write(&mut *writer.lock().await).await?;
                        rooms.insert(name, room);
                    }
                    Err(broadcast::error::RecvError::Closed) => unreachable!("room list should be maintained indefinitely"),
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        let (init, stream) = rooms_handle.stream().await;
                        (room_tx, rooms) = init.clone();
                        room_stream = stream;
                    }
                },
                msg = &mut read => match msg? {
                    LobbyClientMessage::JoinRoom { name, password } => if let Some(room) = rooms.get(&name) {
                        if room.read().await.password != password { error!("wrong password for room {name:?}") }
                        if room.read().await.clients.len() >= u8::MAX.into() { error!("room {name:?} is full") }
                        {
                            let mut room = room.write().await;
                            room.add_client(socket_id, Arc::clone(&writer)).await;
                            let mut players = Vec::<Player>::default();
                            let mut num_unassigned_clients = 0;
                            for &(player, _) in room.clients.values() {
                                if let Some(player) = player {
                                    players.insert(players.binary_search_by_key(&player.world, |p| p.world).expect_err("duplicate world number"), player);
                                } else {
                                    num_unassigned_clients += 1;
                                }
                            }
                            ServerMessage::EnterRoom { players, num_unassigned_clients }.write(&mut *writer.lock().await).await?;
                        }
                        break Arc::clone(room)
                    } else {
                        error!("there is no room named {name:?}")
                    },
                    LobbyClientMessage::CreateRoom { name, password } => {
                        //TODO disallow creating new rooms if preparing for reboot? (or at least warn)
                        if name.is_empty() { error!("room name must not be empty") }
                        if name.chars().count() >= 64 { error!("room name too long (maximum 64 characters)") }
                        if name.contains('\0') { error!("room name must not contain null characters") }
                        if password.chars().count() >= 64 { error!("room password too long (maximum 64 characters)") }
                        if password.contains('\0') { error!("room password must not contain null characters") }
                        if rooms.contains_key(&name) { error!("a room with this name already exists") }
                        let mut clients = HashMap::default();
                        clients.insert(socket_id, (None, Arc::clone(&writer)));
                        let room = Arc::new(RwLock::new(Room {
                            password, clients,
                            base_queue: Vec::default(),
                            player_queues: HashMap::default(),
                        }));
                        room_tx.send(NewRoom { name, room: Arc::clone(&room) }).await.expect("room list should be maintained indefinitely");
                        //TODO automatically delete rooms after 7 days of inactivity (reduce to 24 hours after backup system is implemented, to reduce room list clutter)
                        ServerMessage::EnterRoom {
                            players: Vec::default(),
                            num_unassigned_clients: 1,
                        }.write(&mut *writer.lock().await).await?;
                        break room
                    }
                },
            }
        }
    };
    loop {
        match RoomClientMessage::read(&mut reader).await? {
            RoomClientMessage::PlayerId(id) => if !room.write().await.load_player(socket_id, id).await {
                error!("world {id} is already taken")
            },
            RoomClientMessage::ResetPlayerId => room.write().await.unload_player(socket_id).await,
            RoomClientMessage::PlayerName(name) => if !room.write().await.set_player_name(socket_id, name).await {
                error!("please claim a world before setting your player name")
            },
            RoomClientMessage::SendItem { key, kind, target_world } => if !room.write().await.queue_item(socket_id, key, kind, target_world).await {
                error!("please claim a world before sending items")
            },
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct Rooms;

#[derive(Debug, Clone)]
struct NewRoom {
    name: String,
    room: Arc<RwLock<Room>>,
}

impl<T> ctrlflow::Delta<(T, HashMap<String, Arc<RwLock<Room>>>)> for NewRoom {
    fn apply(&self, state: &mut (T, HashMap<String, Arc<RwLock<Room>>>)) {
        state.1.insert(self.name.clone(), self.room.clone());
    }
}

impl ctrlflow::Key for Rooms {
    type State = (mpsc::Sender<NewRoom>, HashMap<String, Arc<RwLock<Room>>>);
    type Delta = NewRoom;

    fn maintain(self, _: ctrlflow::RunnerInternal<Self>) -> Pin<Box<dyn Future<Output = (Self::State, Pin<Box<dyn Stream<Item = Self::Delta> + Send>>)> + Send>> {
        Box::pin(async move {
            let (tx, rx) = mpsc::channel(64);
            ((tx, HashMap::default()), ReceiverStream::new(rx).boxed())
        })
    }
}

#[wheel::main]
async fn main() -> io::Result<Never> {
    let rooms = ctrlflow::run(Rooms).await;
    let listener = TcpListener::bind((Ipv6Addr::UNSPECIFIED, multiworld::PORT)).await?;
    loop {
        let (socket, _) = listener.accept().await?;
        let socket_id = multiworld::socket_id(&socket);
        let (reader, writer) = socket.into_split();
        let writer = Arc::new(Mutex::new(writer));
        let rooms = rooms.clone();
        tokio::spawn(async move {
            if let Err(e) = client_session(rooms.clone(), socket_id, reader, writer).await {
                eprintln!("{} error in client session: {e:?}", Utc::now().format("%Y-%m-%d %H:%M:%S"));
            }
            for room in rooms.state().await.1.values() {
                if room.read().await.has_client(socket_id) {
                    room.write().await.remove_client(socket_id).await;
                }
            }
        });
    }
}
