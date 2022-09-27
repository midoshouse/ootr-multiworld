#![deny(rust_2018_idioms, unused, unused_crate_dependencies, unused_import_braces, unused_lifetimes, unused_qualifications, warnings)]
#![forbid(unsafe_code)]

use {
    std::{
        collections::{
            BTreeMap,
            HashMap,
        },
        net::Ipv6Addr,
        pin::Pin,
        process,
        sync::Arc,
        time::Duration,
    },
    async_proto::Protocol as _,
    chrono::prelude::*,
    futures::{
        future::Future,
        stream::TryStreamExt as _,
    },
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
        pin,
        select,
        sync::{
            Mutex,
            RwLock,
            broadcast,
            oneshot,
        },
        time::{
            Instant,
            interval_at,
            timeout,
        },
    },
    multiworld::{
        ClientMessage,
        EndRoomSession,
        Player,
        Room,
        ServerError,
        ServerMessage,
    },
};

#[derive(Debug, thiserror::Error)]
enum SessionError {
    #[error(transparent)] Elapsed(#[from] tokio::time::error::Elapsed),
    #[error(transparent)] Read(#[from] async_proto::ReadError),
    #[error(transparent)] OneshotRecv(#[from] oneshot::error::RecvError),
    #[error(transparent)] QueueItem(#[from] multiworld::QueueItemError),
    #[error(transparent)] SendAll(#[from] multiworld::SendAllError),
    #[error(transparent)] SetHashError(#[from] multiworld::SetHashError),
    #[error(transparent)] Sql(#[from] sqlx::Error),
    #[error(transparent)] Write(#[from] async_proto::WriteError),
    #[error("{0}")]
    Server(String),
    #[error("protocol version mismatch: client is version {0} but we're version {}", multiworld::proto_version())]
    VersionMismatch(u8),
}

async fn client_session(db_pool: PgPool, rooms: Rooms, socket_id: multiworld::SocketId, mut reader: OwnedReadHalf, writer: Arc<Mutex<OwnedWriteHalf>>) -> Result<(), SessionError> {
    multiworld::proto_version().write(&mut *writer.lock().await).await?;
    let client_version = u8::read(&mut reader).await?;
    if client_version != multiworld::proto_version() { return Err::<(), _>(SessionError::VersionMismatch(client_version)) }
    let mut read = next_message(reader);
    Ok(loop {
        let (room_reader, room, end_rx) = lobby_session(db_pool.clone(), rooms.clone(), socket_id, read, writer.clone()).await?;
        let (lobby_reader, end) = room_session(db_pool.clone(), rooms.clone(), room, socket_id, room_reader, writer.clone(), end_rx).await?;
        match end {
            EndRoomSession::ToLobby => read = lobby_reader,
            EndRoomSession::Disconnect => {
                ServerMessage::Goodbye.write(&mut *writer.lock().await).await?;
                break
            }
        }
    })
}

type NextMessage = Pin<Box<dyn Future<Output = Result<Result<(OwnedReadHalf, ClientMessage), async_proto::ReadError>, tokio::time::error::Elapsed>> + Send>>;

fn next_message(reader: OwnedReadHalf) -> NextMessage {
    Box::pin(timeout(Duration::from_secs(60), ClientMessage::read_owned(reader)))
}

async fn lobby_session(db_pool: PgPool, rooms: Rooms, socket_id: multiworld::SocketId, mut read: NextMessage, writer: Arc<Mutex<OwnedWriteHalf>>) -> Result<(OwnedReadHalf, Arc<RwLock<Room>>, oneshot::Receiver<EndRoomSession>), SessionError> {
    macro_rules! error {
        ($($msg:tt)*) => {{
            let msg = format!($($msg)*);
            ServerMessage::OtherError(msg.clone()).write(&mut *writer.lock().await).await?;
            return Err(SessionError::Server(msg))
        }};
    }

    let mut logged_in_as_admin = false;
    let mut room_stream = {
        let lock = rooms.0.lock().await;
        let stream = lock.1.subscribe();
        ServerMessage::EnterLobby { rooms: lock.0.keys().cloned().collect() }.write(&mut *writer.lock().await).await?;
        stream
    };
    Ok(loop {
        select! {
            room_list_change = room_stream.recv() => match room_list_change {
                Ok(RoomListChange::New(room)) => ServerMessage::NewRoom(room.read().await.name.clone()).write(&mut *writer.lock().await).await?,
                Ok(RoomListChange::Delete(room_name)) => ServerMessage::DeleteRoom(room_name).write(&mut *writer.lock().await).await?,
                Err(broadcast::error::RecvError::Closed) => unreachable!("room list should be maintained indefinitely"),
                Err(broadcast::error::RecvError::Lagged(_)) => room_stream = rooms.0.lock().await.1.subscribe(),
            },
            res = &mut read => {
                let (reader, msg) = res??;
                match msg {
                    ClientMessage::Ping => {}
                    ClientMessage::JoinRoom { name, password } => if let Some(room) = rooms.0.lock().await.0.get(&name) {
                        if room.read().await.password == password {
                            if room.read().await.clients.len() >= u8::MAX.into() { error!("room {name:?} is full") }
                            let (end_tx, end_rx) = oneshot::channel();
                            {
                                let mut room = room.write().await;
                                room.add_client(socket_id, Arc::clone(&writer), end_tx).await;
                                let mut players = Vec::<Player>::default();
                                let mut num_unassigned_clients = 0;
                                for client in room.clients.values() {
                                    if let Some(player) = client.player {
                                        players.insert(players.binary_search_by_key(&player.world, |p| p.world).expect_err("duplicate world number"), player);
                                    } else {
                                        num_unassigned_clients += 1;
                                    }
                                }
                                ServerMessage::EnterRoom { players, num_unassigned_clients }.write(&mut *writer.lock().await).await?;
                            }
                            break (reader, Arc::clone(room), end_rx)
                        } else {
                            ServerMessage::StructuredError(ServerError::WrongPassword).write(&mut *writer.lock().await).await?;
                        }
                    } else {
                        error!("there is no room named {name:?}")
                    },
                    ClientMessage::CreateRoom { name, password } => {
                        //TODO disallow creating new rooms if preparing for reboot? (or at least warn)
                        if name.is_empty() { error!("room name must not be empty") }
                        if name.chars().count() > 64 { error!("room name too long (maximum 64 characters)") }
                        if name.contains('\0') { error!("room name must not contain null characters") }
                        if password.chars().count() > 64 { error!("room password too long (maximum 64 characters)") }
                        if password.contains('\0') { error!("room password must not contain null characters") }
                        let mut clients = HashMap::default();
                        let (end_tx, end_rx) = oneshot::channel();
                        clients.insert(socket_id, multiworld::Client {
                            player: None,
                            writer: Arc::clone(&writer),
                            save_data: None,
                            end_tx,
                        });
                        let room = Arc::new(RwLock::new(Room {
                            name: name.clone(),
                            file_hash: None,
                            base_queue: Vec::default(),
                            player_queues: HashMap::default(),
                            last_saved: Instant::now(),
                            db_pool: db_pool.clone(),
                            tracker_state: None,
                            password, clients,
                        }));
                        if !rooms.add(room.clone()).await { error!("a room with this name already exists") }
                        //TODO automatically delete rooms after 7 days of inactivity
                        ServerMessage::EnterRoom {
                            players: Vec::default(),
                            num_unassigned_clients: 1,
                        }.write(&mut *writer.lock().await).await?;
                        break (reader, room, end_rx)
                    }
                    ClientMessage::Login { id, api_key } => if id == 14571800683221815449 && api_key == *include_bytes!("../../../assets/admin-api-key.bin") { //TODO allow any Mido's House user to log in but give them different permissions
                        let mut active_connections = BTreeMap::default();
                        for (room_name, room) in &rooms.0.lock().await.0 {
                            let room = room.read().await;
                            let mut players = Vec::<Player>::default();
                            let mut num_unassigned_clients = 0;
                            for client in room.clients.values() {
                                if let Some(player) = client.player {
                                    players.insert(players.binary_search_by_key(&player.world, |p| p.world).expect_err("duplicate world number"), player);
                                } else {
                                    num_unassigned_clients += 1;
                                }
                            }
                            active_connections.insert(room_name.clone(), (players, num_unassigned_clients));
                        }
                        ServerMessage::AdminLoginSuccess { active_connections }.write(&mut *writer.lock().await).await?;
                        logged_in_as_admin = true;
                    } else {
                        error!("wrong user ID or API key")
                    },
                    ClientMessage::Stop => if logged_in_as_admin {
                        //TODO close TCP connections and listener
                        for room in rooms.0.lock().await.0.values() {
                            room.write().await.save().await?;
                        }
                        process::exit(0)
                    } else {
                        error!("Stop command requires admin login")
                    },
                    ClientMessage::Track { mw_room_name, tracker_room_name, world_count } => if logged_in_as_admin {
                        if let Some(room) = rooms.0.lock().await.0.get(&mw_room_name) {
                            room.write().await.init_tracker(tracker_room_name, world_count).await?;
                        } else {
                            error!("there is no room named {mw_room_name:?}")
                        }
                    } else {
                        error!("Track command requires admin login")
                    },
                    ClientMessage::SendAll { room, source_world, spoiler_log } => if logged_in_as_admin {
                        if let Some(room) = rooms.0.lock().await.0.get(&room) {
                            if !room.write().await.send_all(source_world, &spoiler_log).await? {
                                error!("failed to send some items")
                            }
                        } else {
                            error!("there is no room named {room:?}")
                        }
                    } else {
                        error!("SendAll command requires admin login")
                    },
                    ClientMessage::PlayerId(_) |
                    ClientMessage::ResetPlayerId |
                    ClientMessage::PlayerName(_) |
                    ClientMessage::SendItem { .. } |
                    ClientMessage::KickPlayer(_) |
                    ClientMessage::DeleteRoom |
                    ClientMessage::SaveData(_) |
                    ClientMessage::SaveDataError { .. } |
                    ClientMessage::FileHash(_) => error!("received a message that only works in a room, but you're in the lobby"),
                }
                read = next_message(reader);
            }
        }
    })
}

async fn room_session(db_pool: PgPool, rooms: Rooms, room: Arc<RwLock<Room>>, socket_id: multiworld::SocketId, reader: OwnedReadHalf, writer: Arc<Mutex<OwnedWriteHalf>>, mut end_rx: oneshot::Receiver<EndRoomSession>) -> Result<(NextMessage, EndRoomSession), SessionError> {
    macro_rules! error {
        ($($msg:tt)*) => {{
            let msg = format!($($msg)*);
            ServerMessage::OtherError(msg.clone()).write(&mut *writer.lock().await).await?;
            return Err(SessionError::Server(msg))
        }};
    }

    let mut read = next_message(reader);
    Ok(loop {
        select! {
            end_res = &mut end_rx => break (read, end_res?),
            res = &mut read => {
                let (reader, msg) = res??;
                match msg {
                    ClientMessage::Ping => {}
                    ClientMessage::JoinRoom { .. } |
                    ClientMessage::CreateRoom { .. } |
                    ClientMessage::Login { .. } |
                    ClientMessage::Stop |
                    ClientMessage::Track { .. } |
                    ClientMessage::SendAll { .. } => error!("received a message that only works in the lobby, but you're in a room"),
                    ClientMessage::PlayerId(id) => if !room.write().await.load_player(socket_id, id).await? {
                        error!("world {id} is already taken")
                    },
                    ClientMessage::ResetPlayerId => room.write().await.unload_player(socket_id).await,
                    ClientMessage::PlayerName(name) => if !room.write().await.set_player_name(socket_id, name).await {
                        error!("please claim a world before setting your player name")
                    },
                    ClientMessage::SendItem { key, kind, target_world } => if let Err(e) = room.write().await.queue_item(socket_id, key, kind, target_world).await {
                        ServerMessage::OtherError(e.to_string()).write(&mut *writer.lock().await).await?;
                        return Err(e.into())
                    },
                    ClientMessage::KickPlayer(id) => {
                        let mut room = room.write().await;
                        for (&socket_id, client) in &room.clients {
                            if let Some(Player { world, .. }) = client.player {
                                if world == id {
                                    room.remove_client(socket_id, EndRoomSession::ToLobby).await;
                                    break
                                }
                            }
                        }
                    }
                    ClientMessage::DeleteRoom => {
                        let mut room = room.write().await;
                        room.delete().await;
                        rooms.remove(room.name.clone()).await;
                    }
                    ClientMessage::SaveData(save) => room.write().await.set_save_data(socket_id, save).await?,
                    ClientMessage::SaveDataError { debug, version } => if version >= multiworld::version() {
                        sqlx::query!("INSERT INTO save_data_errors (debug, version) VALUES ($1, $2)", debug, version.to_string()).execute(&db_pool).await?;
                    },
                    ClientMessage::FileHash(hash) => if let Err(e) = room.write().await.set_file_hash(socket_id, hash).await {
                        ServerMessage::OtherError(e.to_string()).write(&mut *writer.lock().await).await?;
                        return Err(e.into())
                    },
                }
                read = next_message(reader);
            },
        }
    })
}

#[derive(Clone)]
enum RoomListChange {
    New(Arc<RwLock<Room>>),
    Delete(String),
}

#[derive(Clone)]
struct Rooms(Arc<Mutex<(HashMap<String, Arc<RwLock<Room>>>, broadcast::Sender<RoomListChange>)>>);

impl Rooms {
    async fn add(&self, room: Arc<RwLock<Room>>) -> bool {
        let name = room.read().await.name.clone();
        let mut lock = self.0.lock().await;
        let _ = lock.1.send(RoomListChange::New(room.clone()));
        lock.0.insert(name, room).is_none()
    }

    async fn remove(&self, room_name: String) {
        let mut lock = self.0.lock().await;
        lock.0.remove(&room_name);
        let _ = lock.1.send(RoomListChange::Delete(room_name));
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
    #[clap(short, long, default_value_t = multiworld::PORT)]
    port: u16,
    #[clap(short, long, default_value = "ootr_multiworld")]
    database: String,
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
async fn main(Args { port, database, subcommand }: Args) -> Result<(), Error> {
    match subcommand {
        Some(Subcommand::Stop) => {
            let mut tcp_stream = TcpStream::connect((Ipv6Addr::LOCALHOST, port)).await?;
            multiworld::handshake(&mut tcp_stream).await?;
            ClientMessage::Login { id: 14571800683221815449, api_key: *include_bytes!("../../../assets/admin-api-key.bin") }.write(&mut tcp_stream).await?;
            loop {
                match ServerMessage::read(&mut tcp_stream).await? {
                    ServerMessage::OtherError(msg) => return Err(Error::Server(msg)),
                    ServerMessage::Ping |
                    ServerMessage::EnterLobby { .. } |
                    ServerMessage::NewRoom(_) |
                    ServerMessage::DeleteRoom(_) => {}
                    ServerMessage::AdminLoginSuccess { .. } => break,
                    ServerMessage::StructuredError(ServerError::Future(_)) |
                    ServerMessage::StructuredError(ServerError::WrongPassword) |
                    ServerMessage::EnterRoom { .. } |
                    ServerMessage::PlayerId(_) |
                    ServerMessage::ResetPlayerId(_) |
                    ServerMessage::ClientConnected |
                    ServerMessage::PlayerDisconnected(_) |
                    ServerMessage::UnregisteredClientDisconnected |
                    ServerMessage::PlayerName(_, _) |
                    ServerMessage::ItemQueue(_) |
                    ServerMessage::GetItem(_) |
                    ServerMessage::Goodbye |
                    ServerMessage::PlayerFileHash(_, _) => unreachable!(),
                }
            }
            ClientMessage::Stop.write(&mut tcp_stream).await?;
        }
        None => {
            let listener = TcpListener::bind((Ipv6Addr::UNSPECIFIED, port)).await?;
            let db_pool = PgPool::connect_with(PgConnectOptions::default().username("mido").database(&database).application_name("ootrmwd")).await?;
            let rooms = Rooms::default();
            {
                let mut query = sqlx::query!("SELECT name, password, base_queue, player_queues FROM rooms").fetch(&db_pool);
                while let Some(row) = query.try_next().await? {
                    assert!(rooms.add(Arc::new(RwLock::new(Room {
                        name: row.name.clone(),
                        password: row.password,
                        clients: HashMap::default(),
                        file_hash: None,
                        base_queue: Vec::read_sync(&mut &*row.base_queue)?,
                        player_queues: HashMap::read_sync(&mut &*row.player_queues)?,
                        last_saved: Instant::now(),
                        db_pool: db_pool.clone(),
                        tracker_state: None,
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
                    pin! {
                        let session = client_session(db_pool, rooms.clone(), socket_id, reader, writer.clone());
                    }
                    let mut interval = interval_at(Instant::now() + Duration::from_secs(30), Duration::from_secs(30));
                    loop {
                        select! {
                            res = &mut session => {
                                if let Err(e) = res {
                                    eprintln!("{} error in client session: {e:?}", Utc::now().format("%Y-%m-%d %H:%M:%S"));
                                }
                                break
                            }
                            _ = interval.tick() => if let Err(_) = ServerMessage::Ping.write(&mut *writer.lock().await).await { break },
                        }
                    }
                    for room in rooms.0.lock().await.0.values() {
                        if room.read().await.has_client(socket_id) {
                            room.write().await.remove_client(socket_id, EndRoomSession::Disconnect).await;
                        }
                    }
                });
            }
        }
    }
    Ok(())
}
