#![deny(rust_2018_idioms, unused, unused_crate_dependencies, unused_import_braces, unused_lifetimes, unused_qualifications, warnings)]
#![forbid(unsafe_code)]

use {
    std::{
        collections::{
            BTreeMap,
            HashMap,
        },
        convert::TryFrom as _,
        net::Ipv6Addr,
        process,
        sync::Arc,
        time::Instant,
    },
    async_proto::Protocol as _,
    chrono::prelude::*,
    futures::stream::TryStreamExt as _,
    sqlx::postgres::{
        PgConnectOptions,
        PgPool,
    },
    tokio::{
        io,
        net::{
            TcpListener,
            TcpStream,
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
        },
    },
    multiworld::{
        AdminClientMessage,
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
    #[error(transparent)] Sql(#[from] sqlx::Error),
    #[error(transparent)] Write(#[from] async_proto::WriteError),
    #[error("protocol version mismatch: client is version {0} but we're version {}", multiworld::proto_version())]
    VersionMismatch(u8),
}

async fn client_session(db_pool: PgPool, rooms: Rooms, socket_id: multiworld::SocketId, mut reader: OwnedReadHalf, writer: Arc<Mutex<OwnedWriteHalf>>) -> Result<(), SessionError> {
    macro_rules! error {
        ($($msg:tt)*) => {{
            let msg = format!($($msg)*);
            ServerMessage::OtherError(msg).write(&mut *writer.lock().await).await?;
            return Ok(())
        }};
    }

    multiworld::proto_version().write(&mut *writer.lock().await).await?;
    let client_version = u8::read(&mut reader).await?;
    if client_version != multiworld::proto_version() { return Err::<(), _>(SessionError::VersionMismatch(client_version)) }
    let mut room_stream = {
        // finish handshake by sending room list (treated as a single packet)
        let mut writer = writer.lock().await;
        let lock = rooms.0.lock().await;
        let stream = lock.1.subscribe();
        u64::try_from(lock.0.len()).expect("too many rooms").write(&mut *writer).await?;
        for room_name in lock.0.keys() {
            room_name.write(&mut *writer).await?;
        }
        stream
    };
    let room = {
        let mut read = LobbyClientMessage::read(&mut reader);
        loop {
            select! {
                new_room = room_stream.recv() => match new_room {
                    Ok(room) => ServerMessage::NewRoom(room.read().await.name.clone()).write(&mut *writer.lock().await).await?,
                    Err(broadcast::error::RecvError::Closed) => unreachable!("room list should be maintained indefinitely"),
                    Err(broadcast::error::RecvError::Lagged(_)) => room_stream = rooms.0.lock().await.1.subscribe(),
                },
                msg = &mut read => match msg? {
                    LobbyClientMessage::JoinRoom { name, password } => if let Some(room) = rooms.0.lock().await.0.get(&name) {
                        if room.read().await.password == password {
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
                            ServerMessage::WrongPassword.write(&mut *writer.lock().await).await?;
                        }
                    } else {
                        error!("there is no room named {name:?}")
                    },
                    LobbyClientMessage::CreateRoom { name, password } => {
                        //TODO disallow creating new rooms if preparing for reboot? (or at least warn)
                        if name.is_empty() { error!("room name must not be empty") }
                        if name.chars().count() > 64 { error!("room name too long (maximum 64 characters)") }
                        if name.contains('\0') { error!("room name must not contain null characters") }
                        if password.chars().count() > 64 { error!("room password too long (maximum 64 characters)") }
                        if password.contains('\0') { error!("room password must not contain null characters") }
                        let mut clients = HashMap::default();
                        clients.insert(socket_id, (None, Arc::clone(&writer)));
                        let room = Arc::new(RwLock::new(Room {
                            name: name.clone(),
                            base_queue: Vec::default(),
                            player_queues: HashMap::default(),
                            last_saved: Instant::now(),
                            db_pool: db_pool.clone(),
                            password, clients,
                        }));
                        if !rooms.add(room.clone()).await { error!("a room with this name already exists") }
                        //TODO automatically delete rooms after 7 days of inactivity
                        ServerMessage::EnterRoom {
                            players: Vec::default(),
                            num_unassigned_clients: 1,
                        }.write(&mut *writer.lock().await).await?;
                        break room
                    }
                    LobbyClientMessage::Login { id, api_key } => if id == 14571800683221815449 && api_key == *include_bytes!("../../../assets/admin-api-key.bin") { //TODO allow any Mido's House user to log in but give them different permissions
                        drop(read);
                        let mut active_connections = BTreeMap::default();
                        for (room_name, room) in &rooms.0.lock().await.0 {
                            active_connections.insert(room_name.clone(), room.read().await.clients.len().try_into().expect("more than u8::MAX players in room"));
                        }
                        ServerMessage::AdminLoginSuccess { active_connections }.write(&mut *writer.lock().await).await?;
                        loop {
                            match AdminClientMessage::read(&mut reader).await? {
                                AdminClientMessage::Stop => {
                                    //TODO close TCP connections and listener
                                    for room in rooms.0.lock().await.0.values() {
                                        room.write().await.force_save().await?;
                                    }
                                    process::exit(0)
                                }
                            }
                        }
                    } else {
                        error!("wrong user ID or API key")
                    },
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
            RoomClientMessage::KickPlayer(id) => {
                let mut room = room.write().await;
                for (&socket_id, &(player, _)) in &room.clients {
                    if let Some(Player { world, .. }) = player {
                        if world == id {
                            room.remove_client(socket_id).await;
                            break
                        }
                    }
                }
            }
        }
    }
}

#[derive(Clone)]
struct Rooms(Arc<Mutex<(HashMap<String, Arc<RwLock<Room>>>, broadcast::Sender<Arc<RwLock<Room>>>)>>);

impl Rooms {
    async fn add(&self, room: Arc<RwLock<Room>>) -> bool {
        let name = room.read().await.name.clone();
        let mut lock = self.0.lock().await;
        let _ = lock.1.send(room.clone());
        lock.0.insert(name, room).is_none()
    }
}

impl Default for Rooms {
    fn default() -> Self {
        Self(Arc::new(Mutex::new((HashMap::default(), broadcast::channel(1_024).0))))
    }
}

#[derive(clap::Parser)]
#[clap(version)]
struct Args {
    #[clap(subcommand)]
    subcommand: Option<Subcommand>,
}

#[derive(clap::Subcommand)]
enum Subcommand {
    Stop,
}

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)] Client(#[from] multiworld::ClientError),
    #[error(transparent)] Io(#[from] io::Error),
    #[error(transparent)] Read(#[from] async_proto::ReadError),
    #[error(transparent)] Sql(#[from] sqlx::Error),
    #[error(transparent)] Write(#[from] async_proto::WriteError),
    #[error("server error: {0}")]
    Server(String),
}

#[wheel::main]
async fn main(Args { subcommand }: Args) -> Result<(), Error> {
    match subcommand {
        Some(Subcommand::Stop) => {
            let mut tcp_stream = TcpStream::connect((Ipv6Addr::LOCALHOST, multiworld::PORT)).await?;
            let _ = multiworld::handshake(&mut tcp_stream).await?;
            LobbyClientMessage::Login { id: 14571800683221815449, api_key: *include_bytes!("../../../assets/admin-api-key.bin") }.write(&mut tcp_stream).await?;
            loop {
                match ServerMessage::read(&mut tcp_stream).await? {
                    ServerMessage::OtherError(msg) => return Err(Error::Server(msg)),
                    ServerMessage::NewRoom(_) => {}
                    ServerMessage::AdminLoginSuccess { .. } => break,
                    ServerMessage::EnterRoom { .. } |
                    ServerMessage::PlayerId(_) |
                    ServerMessage::ResetPlayerId(_) |
                    ServerMessage::ClientConnected |
                    ServerMessage::PlayerDisconnected(_) |
                    ServerMessage::UnregisteredClientDisconnected |
                    ServerMessage::PlayerName(_, _) |
                    ServerMessage::ItemQueue(_) |
                    ServerMessage::GetItem(_) |
                    ServerMessage::WrongPassword |
                    ServerMessage::Goodbye => unreachable!(),
                }
            }
            AdminClientMessage::Stop.write(&mut tcp_stream).await?;
        }
        None => {
            let listener = TcpListener::bind((Ipv6Addr::UNSPECIFIED, multiworld::PORT)).await?;
            let db_pool = PgPool::connect_with(PgConnectOptions::default().username("mido").database("ootr_multiworld").application_name("ootrmwd")).await?;
            let rooms = Rooms::default();
            {
                let mut query = sqlx::query!("SELECT name, password, base_queue, player_queues FROM rooms").fetch(&db_pool);
                while let Some(row) = query.try_next().await? {
                    assert!(rooms.add(Arc::new(RwLock::new(Room {
                        name: row.name.clone(),
                        password: row.password,
                        clients: HashMap::default(),
                        base_queue: Vec::read_sync(&mut &*row.base_queue)?,
                        player_queues: HashMap::read_sync(&mut &*row.player_queues)?,
                        last_saved: Instant::now(),
                        db_pool: db_pool.clone(),
                    }))).await);
                }
            }
            loop {
                let (socket, _) = listener.accept().await?;
                let socket_id = multiworld::socket_id(&socket);
                let (reader, writer) = socket.into_split();
                let writer = Arc::new(Mutex::new(writer));
                let db_pool = db_pool.clone();
                let rooms = rooms.clone();
                tokio::spawn(async move {
                    if let Err(e) = client_session(db_pool, rooms.clone(), socket_id, reader, writer).await {
                        eprintln!("{} error in client session: {e:?}", Utc::now().format("%Y-%m-%d %H:%M:%S"));
                    }
                    for room in rooms.0.lock().await.0.values() {
                        if room.read().await.has_client(socket_id) {
                            room.write().await.remove_client(socket_id).await;
                        }
                    }
                });
            }
        }
    }
    Ok(())
}
