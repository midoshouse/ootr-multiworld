#![deny(rust_2018_idioms, unused, unused_crate_dependencies, unused_import_braces, unused_lifetimes, unused_qualifications, warnings)]
#![forbid(unsafe_code)]

use {
    std::{
        collections::{
            BTreeMap,
            HashMap,
        },
        net::Ipv6Addr,
        num::NonZeroU32,
        pin::Pin,
        process,
        sync::Arc,
        time::Duration,
    },
    async_proto::Protocol as _,
    chrono::prelude::*,
    futures::{
        future::{
            self,
            Either,
            Future,
        },
        stream::{
            self,
            StreamExt as _,
            TryStreamExt as _,
        },
    },
    pyo3::prelude::*,
    ring::{
        pbkdf2,
        rand::{
            SecureRandom as _,
            SystemRandom,
        },
    },
    sqlx::postgres::{
        PgConnectOptions,
        PgPool,
        types::PgInterval,
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
            sleep,
            timeout,
        },
    },
    multiworld::{
        CREDENTIAL_LEN,
        ClientMessage,
        EndRoomSession,
        Player,
        Room,
        SendAllError,
        ServerError,
        ServerMessage,
    },
};

#[derive(Debug, thiserror::Error)]
enum PgIntervalDecodeError {
    #[error(transparent)] TryFromInt(#[from] std::num::TryFromIntError),
    #[error("found PgInterval with nonzero months in database")]
    Months,
    #[error("PgInterval too long")]
    Range,
}

fn decode_pginterval(PgInterval { months, days, microseconds }: PgInterval) -> Result<Duration, PgIntervalDecodeError> {
    if months == 0 {
        Duration::from_secs(u64::try_from(days)? * 60 * 60 * 24)
            .checked_add(Duration::from_micros(microseconds.try_into()?))
            .ok_or(PgIntervalDecodeError::Range)
    } else {
        Err(PgIntervalDecodeError::Months)
    }
}

#[derive(Debug, thiserror::Error)]
enum SessionError {
    #[error(transparent)] Elapsed(#[from] tokio::time::error::Elapsed),
    #[error(transparent)] Read(#[from] async_proto::ReadError),
    #[error(transparent)] OneshotRecv(#[from] oneshot::error::RecvError),
    #[error(transparent)] QueueItem(#[from] multiworld::QueueItemError),
    #[error(transparent)] Ring(#[from] ring::error::Unspecified),
    #[error(transparent)] SendAll(#[from] SendAllError),
    #[error(transparent)] SetHashError(#[from] multiworld::SetHashError),
    #[error(transparent)] Sql(#[from] sqlx::Error),
    #[error(transparent)] Write(#[from] async_proto::WriteError),
    #[error("{0}")]
    Server(String),
    #[error("protocol version mismatch: client is version {0} but we're version {}", multiworld::proto_version())]
    VersionMismatch(u8),
}

async fn client_session(rng: &SystemRandom, db_pool: PgPool, rooms: Rooms, socket_id: multiworld::SocketId, mut reader: OwnedReadHalf, writer: Arc<Mutex<OwnedWriteHalf>>) -> Result<(), SessionError> {
    multiworld::proto_version().write(&mut *writer.lock().await).await?;
    let client_version = u8::read(&mut reader).await?;
    if client_version != multiworld::proto_version() { return Err::<(), _>(SessionError::VersionMismatch(client_version)) }
    let mut read = next_message(reader);
    Ok(loop {
        let (room_reader, room, end_rx) = lobby_session(rng, db_pool.clone(), rooms.clone(), socket_id, read, writer.clone()).await?;
        let _ = rooms.0.lock().await.change_tx.send(RoomListChange::Join);
        let (lobby_reader, end) = room_session(db_pool.clone(), rooms.clone(), room, socket_id, room_reader, writer.clone(), end_rx).await?;
        let _ = rooms.0.lock().await.change_tx.send(RoomListChange::Leave);
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

async fn lobby_session(rng: &SystemRandom, db_pool: PgPool, rooms: Rooms, socket_id: multiworld::SocketId, mut read: NextMessage, writer: Arc<Mutex<OwnedWriteHalf>>) -> Result<(OwnedReadHalf, Arc<RwLock<Room>>, oneshot::Receiver<EndRoomSession>), SessionError> {
    macro_rules! error {
        ($($msg:tt)*) => {{
            let msg = format!($($msg)*);
            ServerMessage::OtherError(msg.clone()).write(&mut *writer.lock().await).await?;
            return Err(SessionError::Server(msg))
        }};
    }

    let mut logged_in_as_admin = false;
    let mut waiting_until_empty = false;
    let mut room_stream = {
        let lock = rooms.0.lock().await;
        let stream = lock.change_tx.subscribe();
        ServerMessage::EnterLobby { rooms: lock.list.keys().cloned().collect() }.write(&mut *writer.lock().await).await?;
        stream
    };
    Ok(loop {
        select! {
            room_list_change = room_stream.recv() => {
                match room_list_change {
                    Ok(RoomListChange::New(room)) => ServerMessage::NewRoom(room.read().await.name.clone()).write(&mut *writer.lock().await).await?,
                    Ok(RoomListChange::Delete(room_name)) => ServerMessage::DeleteRoom(room_name).write(&mut *writer.lock().await).await?,
                    Ok(RoomListChange::Join | RoomListChange::Leave) => {}
                    Err(broadcast::error::RecvError::Closed) => unreachable!("room list should be maintained indefinitely"),
                    Err(broadcast::error::RecvError::Lagged(_)) => room_stream = rooms.0.lock().await.change_tx.subscribe(),
                }
                if waiting_until_empty {
                    let mut any_players = false;
                    for room in rooms.0.lock().await.list.values() {
                        if room.read().await.clients.values().any(|client| client.player.is_some()) {
                            any_players = true;
                            break
                        }
                    }
                    if !any_players {
                        ServerMessage::RoomsEmpty.write(&mut *writer.lock().await).await?;
                    }
                }
            }
            res = &mut read => {
                let (reader, msg) = res??;
                match msg {
                    ClientMessage::Ping => {}
                    ClientMessage::JoinRoom { name, password } => if let Some(room_arc) = rooms.0.lock().await.list.get(&name) {
                        let mut room = room_arc.write().await;
                        let authorized = if let Some(password) = password {
                            pbkdf2::verify(
                                pbkdf2::PBKDF2_HMAC_SHA512,
                                NonZeroU32::new(100_000).expect("no hashing iterations specified"),
                                &room.password_salt,
                                password.as_bytes(),
                                &room.password_hash,
                            ).is_ok()
                        } else {
                            logged_in_as_admin
                        };
                        if authorized {
                            if room.clients.len() >= u8::MAX.into() { error!("room {name:?} is full") }
                            let (end_tx, end_rx) = oneshot::channel();
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
                            ServerMessage::EnterRoom {
                                autodelete_delta: room.autodelete_delta,
                                players, num_unassigned_clients,
                            }.write(&mut *writer.lock().await).await?;
                            break (reader, Arc::clone(room_arc), end_rx)
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
                        let mut password_salt = [0; CREDENTIAL_LEN];
                        rng.fill(&mut password_salt)?;
                        let mut password_hash = [0; CREDENTIAL_LEN];
                        pbkdf2::derive(
                            pbkdf2::PBKDF2_HMAC_SHA512,
                            NonZeroU32::new(100_000).expect("no hashing iterations specified"),
                            &password_salt,
                            password.as_bytes(),
                            &mut password_hash,
                        );
                        let mut clients = HashMap::default();
                        let (end_tx, end_rx) = oneshot::channel();
                        clients.insert(socket_id, multiworld::Client {
                            player: None,
                            writer: Arc::clone(&writer),
                            save_data: None,
                            end_tx,
                        });
                        let autodelete_delta = Duration::from_secs(60 * 60 * 24 * 7);
                        let room = Arc::new(RwLock::new(Room {
                            name: name.clone(),
                            file_hash: None,
                            base_queue: Vec::default(),
                            player_queues: HashMap::default(),
                            last_saved: Utc::now(),
                            autodelete_tx: {
                                let rooms = rooms.0.lock().await;
                                rooms.autodelete_tx.clone()
                            },
                            db_pool: db_pool.clone(),
                            tracker_state: None,
                            password_hash, password_salt, clients, autodelete_delta,
                        }));
                        if !rooms.add(room.clone()).await {
                            ServerMessage::StructuredError(ServerError::RoomExists).write(&mut *writer.lock().await).await?;
                        }
                        ServerMessage::EnterRoom {
                            players: Vec::default(),
                            num_unassigned_clients: 1,
                            autodelete_delta,
                        }.write(&mut *writer.lock().await).await?;
                        break (reader, room, end_rx)
                    }
                    ClientMessage::Login { id, api_key } => if id == 14571800683221815449 && api_key == *include_bytes!("../../../assets/admin-api-key.bin") { //TODO allow any Mido's House user to log in but give them different permissions
                        let mut active_connections = BTreeMap::default();
                        for (room_name, room) in &rooms.0.lock().await.list {
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
                        for room in rooms.0.lock().await.list.values() {
                            room.write().await.save(false).await?;
                        }
                        process::exit(0)
                    } else {
                        error!("Stop command requires admin login")
                    },
                    ClientMessage::Track { mw_room_name, tracker_room_name, world_count } => if logged_in_as_admin {
                        if let Some(room) = rooms.0.lock().await.list.get(&mw_room_name) {
                            room.write().await.init_tracker(tracker_room_name, world_count).await?;
                        } else {
                            error!("there is no room named {mw_room_name:?}")
                        }
                    } else {
                        error!("Track command requires admin login")
                    },
                    ClientMessage::WaitUntilEmpty => if logged_in_as_admin {
                        waiting_until_empty = true;
                        let mut any_players = false;
                        for room in rooms.0.lock().await.list.values() {
                            if room.read().await.clients.values().any(|client| client.player.is_some()) {
                                any_players = true;
                                break
                            }
                        }
                        if !any_players {
                            ServerMessage::RoomsEmpty.write(&mut *writer.lock().await).await?;
                        }
                    } else {
                        error!("WaitUntilEmpty command requires admin login")
                    },
                    ClientMessage::PlayerId(_) => error!("received a PlayerId message, which only works in a room, but you're in the lobby"),
                    ClientMessage::ResetPlayerId => error!("received a ResetPlayerId message, which only works in a room, but you're in the lobby"),
                    ClientMessage::PlayerName(_) => error!("received a PlayerName message, which only works in a room, but you're in the lobby"),
                    ClientMessage::SendItem { .. } => error!("received a SendItem message, which only works in a room, but you're in the lobby"),
                    ClientMessage::KickPlayer(_) => error!("received a KickPlayer message, which only works in a room, but you're in the lobby"),
                    ClientMessage::DeleteRoom => error!("received a DeleteRoom message, which only works in a room, but you're in the lobby"),
                    ClientMessage::SaveData(_) => error!("received a SaveData message, which only works in a room, but you're in the lobby"),
                    ClientMessage::SendAll { .. } => error!("received a SendAll message, which only works in a room, but you're in the lobby"),
                    ClientMessage::SaveDataError { .. } => error!("received a SaveDataError message, which only works in a room, but you're in the lobby"),
                    ClientMessage::FileHash(_) => error!("received a FileHash message, which only works in a room, but you're in the lobby"),
                    ClientMessage::AutoDeleteDelta(_) => error!("received an AutoDeleteDelta message, which only works in a room, but you're in the lobby"),
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
                    ClientMessage::JoinRoom { .. } => error!("received a JoinRoom message, which only works in the lobby, but you're in a room"),
                    ClientMessage::CreateRoom { .. } => error!("received a CreateRoom message, which only works in the lobby, but you're in a room"),
                    ClientMessage::Login { .. } => error!("received a Login message, which only works in the lobby, but you're in a room"),
                    ClientMessage::Stop => error!("received a Stop message, which only works in the lobby, but you're in a room"),
                    ClientMessage::Track { .. } => error!("received a Track message, which only works in the lobby, but you're in a room"),
                    ClientMessage::WaitUntilEmpty => error!("received a WaitUntilEmpty message, which only works in the lobby, but you're in a room"),
                    ClientMessage::PlayerId(id) => if !room.write().await.load_player(socket_id, id).await? {
                        error!("world {id} is already taken")
                    },
                    ClientMessage::ResetPlayerId => room.write().await.unload_player(socket_id).await,
                    ClientMessage::PlayerName(name) => if !room.write().await.set_player_name(socket_id, name).await {
                        error!("please claim a world before setting your player name")
                    },
                    ClientMessage::SendItem { key, kind, target_world } => match room.write().await.queue_item(socket_id, key, kind, target_world).await {
                        Ok(()) => {}
                        Err(multiworld::QueueItemError::FileHash) => ServerMessage::StructuredError(ServerError::WrongFileHash).write(&mut *writer.lock().await).await?,
                        Err(e) => {
                            ServerMessage::OtherError(e.to_string()).write(&mut *writer.lock().await).await?;
                            return Err(e.into())
                        }
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
                    ClientMessage::SendAll { source_world, spoiler_log } => room.write().await.send_all(source_world, &spoiler_log).await?,
                    ClientMessage::SaveDataError { debug, version } => if version >= multiworld::version() {
                        sqlx::query!("INSERT INTO save_data_errors (debug, version) VALUES ($1, $2)", debug, version.to_string()).execute(&db_pool).await?;
                    },
                    ClientMessage::FileHash(hash) => match room.write().await.set_file_hash(socket_id, hash).await {
                        Ok(()) => {}
                        Err(multiworld::SetHashError::FileHash) => ServerMessage::StructuredError(ServerError::WrongFileHash).write(&mut *writer.lock().await).await?,
                        Err(e) => {
                            ServerMessage::OtherError(e.to_string()).write(&mut *writer.lock().await).await?;
                            return Err(e.into())
                        }
                    },
                    ClientMessage::AutoDeleteDelta(new_delta) => room.write().await.set_autodelete_delta(new_delta).await,
                }
                read = next_message(reader);
            },
        }
    })
}

#[derive(Clone)]
enum RoomListChange {
    /// A new room has been created.
    New(Arc<RwLock<Room>>),
    /// A room has been deleted.
    Delete(String),
    /// A player has joined a room.
    Join,
    /// A player has left (or been kicked from) a room.
    Leave,
}

struct RoomsInner {
    list: HashMap<String, Arc<RwLock<Room>>>,
    change_tx: broadcast::Sender<RoomListChange>,
    autodelete_tx: broadcast::Sender<(String, DateTime<Utc>)>,
}

#[derive(Clone)]
struct Rooms(Arc<Mutex<RoomsInner>>);

impl Rooms {
    async fn add(&self, room: Arc<RwLock<Room>>) -> bool {
        let name = room.read().await.name.clone();
        let mut lock = self.0.lock().await;
        let _ = lock.change_tx.send(RoomListChange::New(room.clone()));
        lock.list.insert(name, room).is_none()
    }

    async fn remove(&self, room_name: String) {
        let mut lock = self.0.lock().await;
        lock.list.remove(&room_name);
        let _ = lock.change_tx.send(RoomListChange::Delete(room_name));
    }

    async fn wait_cleanup(&self) -> Result<(), broadcast::error::RecvError> {
        let (mut autodelete_at, mut autodelete_rx) = {
            let lock = self.0.lock().await;
            (
                stream::iter(&lock.list).then(|(name, room)| async move { (name.clone(), room.read().await.autodelete_at()) }).collect::<HashMap<_, _>>().await,
                lock.autodelete_tx.subscribe(),
            )
        };
        Ok(loop {
            let now = Utc::now();
            let sleep = if let Some(&time) = autodelete_at.values().min() {
                Either::Left(if let Ok(delta) = (time - now).to_std() {
                    Either::Left(sleep(delta))
                } else {
                    // target time is in the past
                    Either::Right(future::ready(()))
                })
            } else {
                Either::Right(future::pending())
            };
            select! {
                () = sleep => break,
                res = autodelete_rx.recv() => {
                    let (name, time) = res?;
                    autodelete_at.insert(name, time);
                }
            }
        })
    }
}

impl Default for Rooms {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(RoomsInner {
            list: HashMap::default(),
            change_tx: broadcast::channel(1_024).0,
            autodelete_tx: broadcast::channel(1_024).0,
        })))
    }
}

fn parse_port(arg: &str) -> Result<u16, std::num::ParseIntError> {
    match arg {
        "production" => Ok(multiworld::SERVER_PORT),
        "dev" => Ok(18820),
        _ => arg.parse(),
    }
}

#[derive(clap::Parser)]
#[clap(version)]
struct Args {
    #[clap(short, long, default_value_t = multiworld::SERVER_PORT, value_parser = parse_port)]
    port: u16,
    #[clap(short, long, default_value = "ootr_multiworld")]
    database: String,
    #[clap(subcommand)]
    subcommand: Option<Subcommand>,
}

#[derive(clap::Subcommand)]
enum Subcommand {
    Stop,
    StopWhenEmpty,
    WaitUntilEmpty,
}

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)] Broadcast(#[from] broadcast::error::RecvError),
    #[error(transparent)] Client(#[from] multiworld::ClientError),
    #[error(transparent)] Io(#[from] io::Error),
    #[error(transparent)] PgInterval(#[from] PgIntervalDecodeError),
    #[error(transparent)] Read(#[from] async_proto::ReadError),
    #[error(transparent)] Ring(#[from] ring::error::Unspecified),
    #[error(transparent)] Sql(#[from] sqlx::Error),
    #[error(transparent)] Write(#[from] async_proto::WriteError),
    #[error("server error: {0}")]
    Server(String),
}

#[wheel::main]
async fn main(Args { port, database, subcommand }: Args) -> Result<(), Error> {
    if let Some(subcommand) = subcommand {
        let mut tcp_stream = TcpStream::connect((Ipv6Addr::LOCALHOST, port)).await?;
        multiworld::handshake(&mut tcp_stream).await?;
        ClientMessage::Login { id: 14571800683221815449, api_key: *include_bytes!("../../../assets/admin-api-key.bin") }.write(&mut tcp_stream).await?;
        loop {
            match ServerMessage::read(&mut tcp_stream).await? {
                ServerMessage::OtherError(msg) => return Err(Error::Server(msg)),
                ServerMessage::Ping => ClientMessage::Ping.write(&mut tcp_stream).await?,
                ServerMessage::EnterLobby { .. } |
                ServerMessage::NewRoom(_) |
                ServerMessage::DeleteRoom(_) => {}
                ServerMessage::AdminLoginSuccess { .. } => break,
                ServerMessage::StructuredError(ServerError::Future(_)) |
                ServerMessage::StructuredError(ServerError::WrongPassword) |
                ServerMessage::StructuredError(ServerError::WrongFileHash) |
                ServerMessage::StructuredError(ServerError::RoomExists) |
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
                ServerMessage::PlayerFileHash(_, _) |
                ServerMessage::AutoDeleteDelta(_) |
                ServerMessage::RoomsEmpty => unreachable!(),
            }
        }
        if let Subcommand::StopWhenEmpty | Subcommand::WaitUntilEmpty = subcommand {
            ClientMessage::WaitUntilEmpty.write(&mut tcp_stream).await?;
            loop {
                match ServerMessage::read(&mut tcp_stream).await? {
                    ServerMessage::OtherError(msg) => return Err(Error::Server(msg)),
                    ServerMessage::Ping => ClientMessage::Ping.write(&mut tcp_stream).await?,
                    ServerMessage::EnterLobby { .. } |
                    ServerMessage::NewRoom(_) |
                    ServerMessage::DeleteRoom(_) => {}
                    ServerMessage::RoomsEmpty => break,
                    ServerMessage::AdminLoginSuccess { .. } |
                    ServerMessage::StructuredError(ServerError::Future(_)) |
                    ServerMessage::StructuredError(ServerError::WrongPassword) |
                    ServerMessage::StructuredError(ServerError::WrongFileHash) |
                    ServerMessage::StructuredError(ServerError::RoomExists) |
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
                    ServerMessage::PlayerFileHash(_, _) |
                    ServerMessage::AutoDeleteDelta(_) => unreachable!(),
                }
            }
        }
        if let Subcommand::Stop | Subcommand::StopWhenEmpty = subcommand {
            ClientMessage::Stop.write(&mut tcp_stream).await?;
        }
    } else {
        let rng = Arc::new(SystemRandom::new());
        let listener = TcpListener::bind((Ipv6Addr::UNSPECIFIED, port)).await?;
        let db_pool = PgPool::connect_with(PgConnectOptions::default().username("mido").database(&database).application_name("ootrmwd")).await?;
        let rooms = Rooms::default();
        {
            let mut query = sqlx::query!(r#"SELECT
                name,
                password_hash AS "password_hash: [u8; CREDENTIAL_LEN]",
                password_salt AS "password_salt: [u8; CREDENTIAL_LEN]",
                base_queue,
                player_queues,
                last_saved,
                autodelete_delta
            FROM rooms"#).fetch(&db_pool);
            while let Some(row) = query.try_next().await? {
                assert!(rooms.add(Arc::new(RwLock::new(Room {
                    name: row.name.clone(),
                    password_hash: row.password_hash,
                    password_salt: row.password_salt,
                    clients: HashMap::default(),
                    file_hash: None,
                    base_queue: Vec::read_sync(&mut &*row.base_queue)?,
                    player_queues: HashMap::read_sync(&mut &*row.player_queues)?,
                    last_saved: row.last_saved,
                    autodelete_delta: decode_pginterval(row.autodelete_delta)?,
                    autodelete_tx: {
                        let rooms = rooms.0.lock().await;
                        rooms.autodelete_tx.clone()
                    },
                    db_pool: db_pool.clone(),
                    tracker_state: None,
                }))).await);
            }
        }
        loop {
            select! {
                res = listener.accept() => {
                    let (socket, _) = res?;
                    let socket_id = multiworld::socket_id(&socket);
                    let (reader, writer) = socket.into_split();
                    let writer = Arc::new(Mutex::new(writer));
                    let rng = Arc::clone(&rng);
                    let db_pool = db_pool.clone();
                    let rooms = rooms.clone();
                    tokio::spawn(async move {
                        pin! {
                            let session = client_session(&rng, db_pool, rooms.clone(), socket_id, reader, writer.clone());
                        }
                        let mut interval = interval_at(Instant::now() + Duration::from_secs(30), Duration::from_secs(30));
                        loop {
                            select! {
                                res = &mut session => {
                                    match res {
                                        Ok(()) => {}
                                        Err(SessionError::Elapsed(_)) => {} // can be caused by network instability, don't log
                                        Err(SessionError::Read(async_proto::ReadError::Io(e))) if matches!(e.kind(), io::ErrorKind::UnexpectedEof | io::ErrorKind::ConnectionReset) => {} // can be caused by network instability, don't log
                                        Err(SessionError::SendAll(SendAllError::Python(e))) => {
                                            eprintln!("Python error in SendAll command:");
                                            Python::with_gil(|py| e.print(py));
                                        }
                                        Err(e) => eprintln!("error in client session: {e} ({e:?})"),
                                    }
                                    break
                                }
                                _ = interval.tick() => if let Err(_) = ServerMessage::Ping.write(&mut *writer.lock().await).await { break },
                            }
                        }
                        for room in rooms.0.lock().await.list.values() {
                            if room.read().await.has_client(socket_id) {
                                room.write().await.remove_client(socket_id, EndRoomSession::Disconnect).await;
                            }
                        }
                    });
                }
                res = rooms.wait_cleanup() => {
                    let () = res?;
                    let now = Utc::now();
                    while let Some(room) = {
                        let rooms = rooms.0.lock().await;
                        pin! {
                            let rooms_to_delete = stream::iter(rooms.list.values()).filter(|room| async { room.read().await.autodelete_at() <= now });
                        }
                        rooms_to_delete.next().await.cloned()
                    } {
                        let mut room = room.write().await;
                        room.delete().await;
                        rooms.remove(room.name.clone()).await;
                    }
                }
            }
        }
    }
    Ok(())
}
