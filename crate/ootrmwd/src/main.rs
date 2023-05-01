#![deny(rust_2018_idioms, unused, unused_crate_dependencies, unused_import_braces, unused_lifetimes, unused_qualifications, warnings)]
#![forbid(unsafe_code)]

use {
    std::{
        collections::{
            BTreeMap,
            HashMap,
        },
        num::NonZeroU32,
        pin::{
            Pin,
            pin,
        },
        sync::Arc,
        time::Duration,
    },
    async_proto::Protocol as _,
    chrono::prelude::*,
    derivative::Derivative,
    futures::{
        future::{
            self,
            Either,
            Future,
            FutureExt as _,
        },
        stream::{
            self,
            StreamExt as _,
            TryStreamExt as _,
        },
    },
    ring::{
        pbkdf2,
        rand::{
            SecureRandom as _,
            SystemRandom,
        },
    },
    rocket::Rocket,
    sqlx::postgres::{
        PgConnectOptions,
        PgPool,
        types::PgInterval,
    },
    tokio::{
        io,
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
        ClientKind,
        ClientMessage,
        ClientReader as _,
        ClientWriter as _,
        EndRoomSession,
        Player,
        Room,
        SendAllError,
        ServerError,
        ServerMessage,
    },
};
#[cfg(unix)] use {
    tokio::net::UnixStream,
    crate::unix_socket::{
        ClientMessage as Subcommand,
        WaitUntilInactiveMessage,
    },
};

mod http;
#[cfg(unix)] mod unix_socket;

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
    #[error(transparent)] OneshotRecv(#[from] oneshot::error::RecvError),
    #[error(transparent)] QueueItem(#[from] multiworld::QueueItemError),
    #[error(transparent)] Read(#[from] async_proto::ReadError),
    #[error(transparent)] Ring(#[from] ring::error::Unspecified),
    #[error(transparent)] SendAll(#[from] SendAllError),
    #[error(transparent)] SetHash(#[from] multiworld::SetHashError),
    #[error(transparent)] Sql(#[from] sqlx::Error),
    #[error(transparent)] Write(#[from] async_proto::WriteError),
    #[error("{0}")]
    Server(String),
    #[error("server is shutting down")]
    Shutdown,
}

async fn client_session<C: ClientKind>(rng: &SystemRandom, db_pool: PgPool, rooms: Rooms<C>, socket_id: C::SessionId, reader: C::Reader, writer: Arc<Mutex<C::Writer>>, shutdown: rocket::Shutdown) -> Result<(), SessionError> {
    let ping_writer = Arc::clone(&writer);
    let ping_task = tokio::spawn(async move {
        let mut interval = interval_at(Instant::now() + Duration::from_secs(30), Duration::from_secs(30));
        loop {
            interval.tick().await;
            if let Err(_) = ping_writer.lock().await.write(&ServerMessage::Ping).await { break }
        }
    });
    let mut read = next_message::<C>(reader);
    loop {
        let (room_reader, room, end_rx) = lobby_session(rng, db_pool.clone(), rooms.clone(), socket_id, read, writer.clone(), shutdown.clone()).await?;
        let _ = rooms.0.lock().await.change_tx.send(RoomListChange::Join);
        let (lobby_reader, end) = room_session(db_pool.clone(), rooms.clone(), room, socket_id, room_reader, writer.clone(), end_rx, shutdown.clone()).await?;
        let _ = rooms.0.lock().await.change_tx.send(RoomListChange::Leave);
        match end {
            EndRoomSession::ToLobby => read = lobby_reader,
            EndRoomSession::Disconnect => {
                writer.lock().await.write(&ServerMessage::Goodbye).await?;
                break
            }
        }
    }
    ping_task.abort();
    Ok(())
}

type NextMessage<C> = Pin<Box<dyn Future<Output = Result<Result<(<C as ClientKind>::Reader, ClientMessage), async_proto::ReadError>, tokio::time::error::Elapsed>> + Send>>;

fn next_message<C: ClientKind>(reader: C::Reader) -> NextMessage<C> {
    Box::pin(timeout(Duration::from_secs(60), reader.read_owned()))
}

async fn lobby_session<C: ClientKind>(rng: &SystemRandom, db_pool: PgPool, rooms: Rooms<C>, socket_id: C::SessionId, mut read: NextMessage<C>, writer: Arc<Mutex<C::Writer>>, mut shutdown: rocket::Shutdown) -> Result<(C::Reader, Arc<RwLock<Room<C>>>, oneshot::Receiver<EndRoomSession>), SessionError> {
    macro_rules! error {
        ($($msg:tt)*) => {{
            let msg = format!($($msg)*);
            writer.lock().await.write(&ServerMessage::OtherError(msg.clone())).await?;
            return Err(SessionError::Server(msg))
        }};
    }

    let mut logged_in_as_admin = false;
    let mut waiting_until_empty = false;
    let mut room_stream = {
        let lock = rooms.0.lock().await;
        let stream = lock.change_tx.subscribe();
        writer.lock().await.write(&ServerMessage::EnterLobby { rooms: lock.list.keys().cloned().collect() }).await?;
        stream
    };
    Ok(loop {
        select! {
            () = &mut shutdown => return Err(SessionError::Shutdown),
            room_list_change = room_stream.recv() => {
                match room_list_change {
                    Ok(RoomListChange::New(room)) => writer.lock().await.write(&ServerMessage::NewRoom(room.read().await.name.clone())).await?,
                    Ok(RoomListChange::Delete(room_name)) => writer.lock().await.write(&ServerMessage::DeleteRoom(room_name)).await?,
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
                        writer.lock().await.write(&ServerMessage::RoomsEmpty).await?;
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
                            if room.clients.len() >= usize::from(u8::MAX) { error!("room {name:?} is full") }
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
                            writer.lock().await.write(&ServerMessage::EnterRoom {
                                autodelete_delta: room.autodelete_delta,
                                players, num_unassigned_clients,
                            }).await?;
                            break (reader, Arc::clone(room_arc), end_rx)
                        } else {
                            writer.lock().await.write(&ServerMessage::StructuredError(ServerError::WrongPassword)).await?;
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
                            writer.lock().await.write(&ServerMessage::StructuredError(ServerError::RoomExists)).await?;
                        }
                        writer.lock().await.write(&ServerMessage::EnterRoom {
                            players: Vec::default(),
                            num_unassigned_clients: 1,
                            autodelete_delta,
                        }).await?;
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
                        writer.lock().await.write(&ServerMessage::AdminLoginSuccess { active_connections }).await?;
                        logged_in_as_admin = true;
                    } else {
                        error!("wrong user ID or API key")
                    },
                    ClientMessage::Stop => if logged_in_as_admin {
                        //TODO close TCP connections and listener
                        for room in rooms.0.lock().await.list.values() {
                            room.write().await.save(false).await?;
                        }
                        shutdown.notify();
                        return Err(SessionError::Shutdown)
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
                            writer.lock().await.write(&ServerMessage::RoomsEmpty).await?;
                        }
                    } else {
                        error!("WaitUntilEmpty command requires admin login")
                    },
                    ClientMessage::PlayerId(_) => error!("received a PlayerId message, which only works in a room, but you're in the lobby"),
                    ClientMessage::ResetPlayerId => error!("received a ResetPlayerId message, which only works in a room, but you're in the lobby"),
                    ClientMessage::PlayerName(_) => error!("received a PlayerName message, which only works in a room, but you're in the lobby"),
                    ClientMessage::SendItem { .. } => error!("received a SendItem message, which only works in a room, but you're in the lobby"),
                    ClientMessage::KickPlayer(_) => {}
                    ClientMessage::DeleteRoom => {}
                    ClientMessage::SaveData(_) => error!("received a SaveData message, which only works in a room, but you're in the lobby"),
                    ClientMessage::SendAll { .. } => error!("received a SendAll message, which only works in a room, but you're in the lobby"),
                    ClientMessage::SaveDataError { .. } => error!("received a SaveDataError message, which only works in a room, but you're in the lobby"),
                    ClientMessage::FileHash(_) => error!("received a FileHash message, which only works in a room, but you're in the lobby"),
                    ClientMessage::AutoDeleteDelta(_) => error!("received an AutoDeleteDelta message, which only works in a room, but you're in the lobby"),
                }
                read = next_message::<C>(reader);
            }
        }
    })
}

async fn room_session<C: ClientKind>(db_pool: PgPool, rooms: Rooms<C>, room: Arc<RwLock<Room<C>>>, socket_id: C::SessionId, reader: C::Reader, writer: Arc<Mutex<C::Writer>>, mut end_rx: oneshot::Receiver<EndRoomSession>, mut shutdown: rocket::Shutdown) -> Result<(NextMessage<C>, EndRoomSession), SessionError> {
    macro_rules! error {
        ($($msg:tt)*) => {{
            let msg = format!($($msg)*);
            writer.lock().await.write(&ServerMessage::OtherError(msg.clone())).await?;
            return Err(SessionError::Server(msg))
        }};
    }

    let mut read = next_message::<C>(reader);
    Ok(loop {
        select! {
            () = &mut shutdown => return Err(SessionError::Shutdown),
            end_res = &mut end_rx => break (read, end_res?),
            res = &mut read => {
                let (reader, msg) = res??;
                match msg {
                    ClientMessage::Ping => {}
                    ClientMessage::JoinRoom { name, .. } => if name != room.read().await.name {
                        error!("received a JoinRoom message, which only works in the lobby, but you're in a room")
                    }
                    ClientMessage::CreateRoom { name, .. } => if name != room.read().await.name {
                        error!("received a CreateRoom message, which only works in the lobby, but you're in a room")
                    }
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
                        Err(multiworld::QueueItemError::FileHash { server, client }) => writer.lock().await.write(&ServerMessage::WrongFileHash { server, client }).await?,
                        Err(e) => {
                            writer.lock().await.write(&ServerMessage::OtherError(e.to_string())).await?;
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
                    ClientMessage::SendAll { source_world, spoiler_log } => match room.write().await.send_all(source_world, &spoiler_log).await {
                        Ok(()) => {}
                        Err(multiworld::SendAllError::FileHash { server, client }) => writer.lock().await.write(&ServerMessage::WrongFileHash { server, client }).await?,
                        Err(e) => {
                            writer.lock().await.write(&ServerMessage::OtherError(e.to_string())).await?;
                            return Err(e.into())
                        }
                    },
                    ClientMessage::SaveDataError { debug, version } => if version >= multiworld::version() {
                        sqlx::query!("INSERT INTO save_data_errors (debug, version) VALUES ($1, $2)", debug, version.to_string()).execute(&db_pool).await?;
                    },
                    ClientMessage::FileHash(hash) => match room.write().await.set_file_hash(socket_id, hash).await {
                        Ok(()) => {}
                        Err(multiworld::SetHashError::FileHash { server, client }) => writer.lock().await.write(&ServerMessage::WrongFileHash { server, client }).await?,
                        Err(e) => {
                            writer.lock().await.write(&ServerMessage::OtherError(e.to_string())).await?;
                            return Err(e.into())
                        }
                    },
                    ClientMessage::AutoDeleteDelta(new_delta) => room.write().await.set_autodelete_delta(new_delta).await,
                }
                read = next_message::<C>(reader);
            },
        }
    })
}

#[derive(Derivative)]
#[derivative(Clone(bound = ""))]
enum RoomListChange<C: ClientKind> {
    /// A new room has been created.
    New(Arc<RwLock<Room<C>>>),
    /// A room has been deleted.
    Delete(String),
    /// A player has joined a room.
    Join,
    /// A player has left (or been kicked from) a room.
    Leave,
}

struct RoomsInner<C: ClientKind> {
    list: HashMap<String, Arc<RwLock<Room<C>>>>,
    change_tx: broadcast::Sender<RoomListChange<C>>,
    autodelete_tx: broadcast::Sender<(String, DateTime<Utc>)>,
    #[cfg(unix)]
    inactive_tx: broadcast::Sender<(String, DateTime<Utc>)>,
}

#[derive(Derivative)]
#[derivative(Clone(bound = ""))]
struct Rooms<C: ClientKind>(Arc<Mutex<RoomsInner<C>>>);

impl<C: ClientKind> Rooms<C> {
    async fn add(&self, room: Arc<RwLock<Room<C>>>) -> bool {
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

    async fn wait_cleanup(&self, mut shutdown: rocket::Shutdown) -> Result<(), broadcast::error::RecvError> {
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
                () = &mut shutdown => break,
                () = sleep => break,
                res = autodelete_rx.recv() => {
                    let (name, time) = res?;
                    autodelete_at.insert(name, time);
                }
            }
        })
    }

    #[cfg(unix)]
    async fn wait_inactive(&self, mut shutdown: rocket::Shutdown) -> Result<(), broadcast::error::RecvError> {
        let (mut inactive_at, mut inactive_rx) = {
            let lock = self.0.lock().await;
            (
                stream::iter(&lock.list).then(|(name, room)| async move { (name.clone(), room.read().await.last_saved + chrono::Duration::hours(1)) }).collect::<HashMap<_, _>>().await,
                lock.inactive_tx.subscribe(),
            )
        };
        Ok(loop {
            let now = Utc::now();
            let sleep = if let Some(&time) = inactive_at.values().min() {
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
                () = &mut shutdown => break,
                () = sleep => break,
                res = inactive_rx.recv() => {
                    let (name, time) = res?;
                    inactive_at.insert(name, time);
                }
            }
        })
    }
}

impl<C: ClientKind> Default for Rooms<C> {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(RoomsInner {
            list: HashMap::default(),
            change_tx: broadcast::channel(1_024).0,
            autodelete_tx: broadcast::channel(1_024).0,
            #[cfg(unix)]
            inactive_tx: broadcast::channel(1_024).0,
        })))
    }
}

#[derive(clap::Parser)]
#[clap(version)]
struct Args {
    #[clap(short, long, default_value = "ootr_multiworld")]
    database: String,
    #[clap(short, long, default_value = "24819")]
    port: u16,
    #[clap(subcommand)]
    subcommand: Option<Subcommand>,
}

#[cfg(not(unix))]
#[derive(clap::Subcommand)]
enum Subcommand {}

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)] Broadcast(#[from] broadcast::error::RecvError),
    #[error(transparent)] Client(#[from] multiworld::ClientError),
    #[error(transparent)] Io(#[from] io::Error),
    #[error(transparent)] PgInterval(#[from] PgIntervalDecodeError),
    #[error(transparent)] Read(#[from] async_proto::ReadError),
    #[error(transparent)] Ring(#[from] ring::error::Unspecified),
    #[error(transparent)] Rocket(#[from] rocket::Error),
    #[error(transparent)] Sql(#[from] sqlx::Error),
    #[error(transparent)] Task(#[from] tokio::task::JoinError),
    #[cfg(unix)] #[error(transparent)] Wheel(#[from] wheel::Error),
    #[error(transparent)] Write(#[from] async_proto::WriteError),
    #[cfg(unix)]
    #[error("error while waiting until inactive")]
    WaitUntilInactive,
}

#[wheel::main(debug, rocket)]
async fn main(Args { database, port, subcommand }: Args) -> Result<(), Error> {
    if let Some(subcommand) = subcommand {
        #[cfg(unix)] {
            let mut sock = UnixStream::connect(unix_socket::PATH).await?;
            subcommand.write(&mut sock).await?;
            match subcommand {
                Subcommand::Stop | Subcommand::StopWhenEmpty | Subcommand::WaitUntilEmpty => { u8::read(&mut sock).await?; }
                Subcommand::WaitUntilInactive => loop {
                    match WaitUntilInactiveMessage::read(&mut sock).await? {
                        WaitUntilInactiveMessage::Error => return Err(Error::WaitUntilInactive),
                        WaitUntilInactiveMessage::ActiveRooms(rooms) => {
                            wheel::print_flush!(
                                "\r[....] waiting for {} rooms to be inactive (current ETA: {}) ",
                                rooms.len(),
                                rooms.values().map(|(inactive_at, _)| inactive_at).max().expect("waiting for 0 rooms").format("%Y-%m-%d %H:%M:%S UTC"),
                            )?;
                        }
                        WaitUntilInactiveMessage::Inactive => {
                            println!("[ ok ]");
                            break
                        }
                    }
                },
            }
            return Ok(())
        }
        #[cfg(not(unix))] match subcommand {}
    } else {
        let rng = Arc::new(SystemRandom::new());
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
        let rocket = http::rocket(db_pool.clone(), rng.clone(), port, rooms.clone()).await?;
        #[cfg(unix)] let unix_socket_task = tokio::spawn(unix_socket::listen(rocket.shutdown(), rooms.clone())).map(|res| match res {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(Error::from(e)),
            Err(e) => Err(Error::from(e)),
        });
        #[cfg(not(unix))] let unix_socket_task = future::ok(());
        let shutdown = rocket.shutdown();
        let cleanup_task = tokio::spawn(async move {
            loop {
                rooms.wait_cleanup(shutdown.clone()).await?;
                let now = Utc::now();
                while let Some(room) = {
                    let rooms = rooms.0.lock().await;
                    let mut rooms_to_delete = pin!(stream::iter(rooms.list.values()).filter(|room| async { room.read().await.autodelete_at() <= now }));
                    rooms_to_delete.next().await.cloned()
                } {
                    let mut room = room.write().await;
                    room.delete().await;
                    rooms.remove(room.name.clone()).await;
                }
            }
        }).map(|res| match res {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(e) => Err(Error::from(e)),
        });
        let rocket_task = tokio::spawn(rocket.launch()).map(|res| match res {
            Ok(Ok(Rocket { .. })) => Ok(()),
            Ok(Err(e)) => Err(Error::from(e)),
            Err(e) => Err(Error::from(e)),
        });
        let ((), (), ()) = tokio::try_join!(unix_socket_task, cleanup_task, rocket_task)?;
        Ok(())
    }
}
