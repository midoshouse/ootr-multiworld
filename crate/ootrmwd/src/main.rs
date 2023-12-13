#![deny(rust_2018_idioms, unused, unused_crate_dependencies, unused_import_braces, unused_lifetimes, unused_qualifications, warnings)]
#![forbid(unsafe_code)]

use {
    std::{
        collections::hash_map::{
            self,
            HashMap,
        },
        mem,
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
    either::Either,
    futures::{
        future::{
            self,
            Either as EitherFuture,
            Future,
            FutureExt as _,
        },
        stream::{
            self,
            StreamExt as _,
            TryStreamExt as _,
        },
    },
    log_lock::{
        ArcRwLock,
        Mutex,
        OwnedRwLockWriteGuard,
        lock,
    },
    rand::prelude::*,
    ring::{
        pbkdf2,
        rand::{
            SecureRandom as _,
            SystemRandom,
        },
    },
    rocket::Rocket,
    serde::Deserialize,
    sqlx::postgres::{
        PgConnectOptions,
        PgPool,
        types::PgInterval,
    },
    tokio::{
        io,
        process::Command,
        select,
        sync::{
            broadcast,
            oneshot,
            watch,
        },
        time::{
            Instant,
            interval_at,
            sleep,
            timeout,
        },
    },
    wheel::traits::ReqwestResponseExt as _,
    multiworld::{
        CREDENTIAL_LEN,
        ClientKind,
        ClientReader as _,
        ClientWriter as _,
        EndRoomSession,
        Player,
        Room,
        RoomAuth,
        RoomAvailability,
        SendAllError,
        ws::{
            ServerError,
            unversioned::{
                ClientMessage,
                ServerMessage,
            },
        },
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
    #[error(transparent)] Reqwest(#[from] reqwest::Error),
    #[error(transparent)] Ring(#[from] ring::error::Unspecified),
    #[error(transparent)] SendAll(#[from] SendAllError),
    #[error(transparent)] SetHash(#[from] multiworld::SetHashError),
    #[error(transparent)] Sql(#[from] sqlx::Error),
    #[error(transparent)] Wheel(#[from] wheel::Error),
    #[error(transparent)] Write(#[from] async_proto::WriteError),
    #[error("{0}")]
    Server(String),
    #[error("server is shutting down")]
    Shutdown,
}

async fn client_session<C: ClientKind>(rng: &SystemRandom, db_pool: PgPool, http_client: reqwest::Client, rooms: Rooms<C>, socket_id: C::SessionId, reader: C::Reader, writer: Arc<Mutex<C::Writer>>, shutdown: rocket::Shutdown, maintenance: Arc<watch::Sender<Option<(DateTime<Utc>, Duration)>>>) -> Result<(), SessionError> {
    let mut maintenance = maintenance.subscribe();
    let ping_writer = Arc::clone(&writer);
    let ping_task = tokio::spawn(async move {
        let mut interval = interval_at(Instant::now() + Duration::from_secs(30), Duration::from_secs(30));
        loop {
            interval.tick().await;
            if let Err(_) = lock!(ping_writer).write(ServerMessage::Ping).await { break }
        }
    });
    let mut read = next_message::<C>(reader);
    let mut logged_in_as_admin = false;
    let mut midos_house_user_id = None;
    loop {
        let (room_reader, room, end_rx) = lobby_session(rng, db_pool.clone(), http_client.clone(), rooms.clone(), socket_id, read, writer.clone(), shutdown.clone(), &mut maintenance, &mut logged_in_as_admin, &mut midos_house_user_id).await?;
        let _ = lock!(rooms.0).change_tx.send(RoomListChange::Join);
        let (lobby_reader, end) = room_session(rooms.clone(), room, socket_id, room_reader, writer.clone(), &mut maintenance, end_rx, shutdown.clone()).await?;
        let _ = lock!(rooms.0).change_tx.send(RoomListChange::Leave);
        match end {
            EndRoomSession::ToLobby => read = lobby_reader,
            EndRoomSession::Disconnect => {
                lock!(writer).write(ServerMessage::Goodbye).await?;
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

async fn lobby_session<C: ClientKind>(rng: &SystemRandom, db_pool: PgPool, http_client: reqwest::Client, rooms: Rooms<C>, socket_id: C::SessionId, mut read: NextMessage<C>, writer: Arc<Mutex<C::Writer>>, mut shutdown: rocket::Shutdown, maintenance: &mut watch::Receiver<Option<(DateTime<Utc>, Duration)>>, logged_in_as_admin: &mut bool, midos_house_user_id: &mut Option<u64>) -> Result<(C::Reader, ArcRwLock<Room<C>>, oneshot::Receiver<EndRoomSession>), SessionError> {
    macro_rules! error {
        ($($msg:tt)*) => {{
            return Err(SessionError::Server(format!($($msg)*)))
        }};
    }

    let mut waiting_until_empty = false;
    let mut room_stream = {
        let lock = lock!(rooms.0);
        let stream = lock.change_tx.subscribe();
        let logged_in_as_admin = *logged_in_as_admin;
        let midos_house_user_id = *midos_house_user_id;
        lock!(writer).write(ServerMessage::EnterLobby {
            rooms: stream::iter(lock.list.iter()).filter_map(|(id, room)| async move {
                let room = lock!(@read room);
                let password_required = match room.auth.availability(logged_in_as_admin, midos_house_user_id) {
                    RoomAvailability::Open => false,
                    RoomAvailability::PasswordRequired => true,
                    RoomAvailability::Invisible => return None,
                };
                Some((*id, (room.name.clone(), password_required)))
            }).collect().await,
        }).await?;
        stream
    };
    Ok(loop {
        select! {
            () = &mut shutdown => return Err(SessionError::Shutdown),
            Ok(()) = maintenance.changed() => {
                let maintenance = *maintenance.borrow_and_update();
                if let Some((start, duration)) = maintenance {
                    lock!(writer).write(ServerMessage::MaintenanceNotice { start, duration }).await?;
                }
            }
            room_list_change = room_stream.recv() => {
                match room_list_change {
                    Ok(RoomListChange::New(room)) => {
                        let room = lock!(@read room);
                        let password_required = match room.auth.availability(*logged_in_as_admin, *midos_house_user_id) {
                            RoomAvailability::Open => Some(false),
                            RoomAvailability::PasswordRequired => Some(true),
                            RoomAvailability::Invisible => None, // don't announce the room to the client
                        };
                        if let Some(password_required) = password_required {
                            lock!(writer).write(ServerMessage::NewRoom {
                                id: room.id,
                                name: room.name.clone(),
                                password_required,
                            }).await?;
                        }
                    }
                    Ok(RoomListChange::Delete { id, name, auth }) => {
                        let visible = match auth.availability(*logged_in_as_admin, *midos_house_user_id) {
                            RoomAvailability::Open | RoomAvailability::PasswordRequired => true,
                            RoomAvailability::Invisible => false,
                        };
                        if visible {
                            lock!(writer).write(ServerMessage::DeleteRoom { id, name }).await?;
                        }
                    }
                    Ok(RoomListChange::Join | RoomListChange::Leave) => {}
                    Err(broadcast::error::RecvError::Closed) => unreachable!("room list should be maintained indefinitely"),
                    Err(broadcast::error::RecvError::Lagged(_)) => room_stream = lock!(rooms.0).change_tx.subscribe(),
                }
                if waiting_until_empty {
                    let mut any_players = false;
                    for room in lock!(rooms.0).list.values() {
                        if lock!(@read room).clients.values().any(|client| client.player.is_some()) {
                            any_players = true;
                            break
                        }
                    }
                    if !any_players {
                        lock!(writer).write(ServerMessage::RoomsEmpty).await?;
                    }
                }
            }
            res = &mut read => {
                let (reader, msg) = res??;
                match msg {
                    ClientMessage::Ping => {}
                    ClientMessage::JoinRoom { ref room, password } => if let Some(room_arc) = rooms.get_arc(room).await {
                        let mut room = lock!(@write room_arc);
                        let authorized = *logged_in_as_admin || match &room.auth {
                            RoomAuth::Password { hash, salt } => password.map_or(false, |password| pbkdf2::verify(
                                pbkdf2::PBKDF2_HMAC_SHA512,
                                NonZeroU32::new(100_000).expect("no hashing iterations specified"),
                                salt,
                                password.as_bytes(),
                                hash,
                            ).is_ok()),
                            RoomAuth::Invitational(users) => midos_house_user_id.map_or(false, |user| users.contains(&user)),
                        };
                        if authorized {
                            if room.clients.len() >= usize::from(u8::MAX) { error!("this room is full") }
                            let (end_tx, end_rx) = oneshot::channel();
                            room.add_client(socket_id, Arc::clone(&writer), end_tx).await?;
                            let mut players = Vec::<Player>::default();
                            let mut num_unassigned_clients = 0;
                            for client in room.clients.values() {
                                if let Some(player) = client.player {
                                    players.insert(players.binary_search_by_key(&player.world, |p| p.world).expect_err("duplicate world number"), player);
                                } else {
                                    num_unassigned_clients += 1;
                                }
                            }
                            lock!(writer).write(ServerMessage::EnterRoom {
                                room_id: room.id,
                                autodelete_delta: room.autodelete_delta,
                                allow_send_all: room.allow_send_all,
                                players, num_unassigned_clients,
                            }).await?;
                            drop(room);
                            break (reader, room_arc, end_rx)
                        } else {
                            lock!(writer).write(ServerMessage::StructuredError(ServerError::WrongPassword)).await?;
                        }
                    } else {
                        match room {
                            Either::Left(_) => error!("there is no room with this ID"),
                            Either::Right(name) => error!("there is no room named {name:?}"),
                        }
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
                            pending_world: None,
                            pending_name: None,
                            pending_hash: None,
                            writer: Arc::clone(&writer),
                            save_data: None,
                            adjusted_save: Default::default(),
                            end_tx,
                        });
                        let autodelete_delta = Duration::from_secs(60 * 60 * 24 * 7);
                        let id = loop {
                            let id = thread_rng().gen::<u64>();
                            if !sqlx::query_scalar!(r#"SELECT EXISTS (SELECT 1 FROM mw_rooms WHERE id = $1) AS "exists!""#, id as i64).fetch_one(&db_pool).await? { break id } //TODO save room to database in same transaction
                        };
                        let room = ArcRwLock::new(Room {
                            name: name.clone(),
                            auth: RoomAuth::Password {
                                hash: password_hash,
                                salt: password_salt,
                            },
                            file_hash: None,
                            base_queue: Vec::default(),
                            player_queues: HashMap::default(),
                            last_saved: Utc::now(),
                            allow_send_all: true,
                            autodelete_tx: {
                                let rooms = lock!(rooms.0);
                                rooms.autodelete_tx.clone()
                            },
                            db_pool: db_pool.clone(),
                            tracker_state: None,
                            id, clients, autodelete_delta,
                        });
                        if !rooms.add(room.clone()).await {
                            lock!(writer).write(ServerMessage::StructuredError(ServerError::RoomExists)).await?;
                        }
                        lock!(writer).write(ServerMessage::EnterRoom {
                            room_id: id,
                            players: Vec::default(),
                            num_unassigned_clients: 1,
                            allow_send_all: true,
                            autodelete_delta,
                        }).await?;
                        break (reader, room, end_rx)
                    }
                    ClientMessage::LoginApiKey { api_key } => if let Some(row) = sqlx::query!("SELECT user_id, mw_admin FROM api_keys WHERE key = $1", api_key).fetch_optional(&db_pool).await? {
                        lock!(writer).write(ServerMessage::LoginSuccess).await?;
                        let was_admin = mem::replace(logged_in_as_admin, row.mw_admin);
                        let old_mhid = midos_house_user_id.replace(row.user_id as u64);
                        update_room_list(rooms.clone(), Arc::clone(&writer), was_admin, old_mhid, *logged_in_as_admin, *midos_house_user_id).await?;
                    } else {
                        error!("invalid API key")
                    },
                    ClientMessage::LoginDiscord { bearer_token } => {
                        match http_client.get("https://discord.com/api/v10/users/@me").bearer_auth(bearer_token).send().await?.detailed_error_for_status().await {
                            Ok(response) => {
                                #[derive(Deserialize)]
                                struct DiscordUser {
                                    id: serenity::all::UserId,
                                }

                                let DiscordUser { id } = response.json_with_text_in_error().await?;
                                if let Some(mhid) = sqlx::query_scalar!("SELECT id FROM users WHERE discord_id = $1", i64::from(id)).fetch_optional(&db_pool).await? {
                                    lock!(writer).write(ServerMessage::LoginSuccess).await?;
                                    let old_mhid = midos_house_user_id.replace(mhid as u64);
                                    update_room_list(rooms.clone(), Arc::clone(&writer), *logged_in_as_admin, old_mhid, *logged_in_as_admin, *midos_house_user_id).await?;
                                } else {
                                    lock!(writer).write(ServerMessage::StructuredError(ServerError::NoMidosHouseAccountDiscord)).await?; //TODO automatically create
                                }
                            }
                            Err(wheel::Error::ResponseStatus { inner, .. }) if inner.status() == Some(reqwest::StatusCode::UNAUTHORIZED) => {
                                lock!(writer).write(ServerMessage::StructuredError(ServerError::SessionExpiredDiscord)).await?;
                            }
                            Err(e) => return Err(e.into()),
                        }
                    }
                    ClientMessage::LoginRaceTime { bearer_token } => {
                        match http_client.get("https://racetime.gg/o/userinfo").bearer_auth(bearer_token).send().await?.detailed_error_for_status().await {
                            Ok(response) => {
                                #[derive(Deserialize)]
                                struct RaceTimeUser {
                                    id: String,
                                }

                                let RaceTimeUser { id } = response.json_with_text_in_error().await?;
                                if let Some(mhid) = sqlx::query_scalar!("SELECT id FROM users WHERE racetime_id = $1", id).fetch_optional(&db_pool).await? {
                                    lock!(writer).write(ServerMessage::LoginSuccess).await?;
                                    let old_mhid = midos_house_user_id.replace(mhid as u64);
                                    update_room_list(rooms.clone(), Arc::clone(&writer), *logged_in_as_admin, old_mhid, *logged_in_as_admin, *midos_house_user_id).await?;
                                } else {
                                    lock!(writer).write(ServerMessage::StructuredError(ServerError::NoMidosHouseAccountRaceTime)).await?; //TODO automatically create
                                }
                            }
                            Err(wheel::Error::ResponseStatus { inner, .. }) if inner.status() == Some(reqwest::StatusCode::FORBIDDEN) => {
                                lock!(writer).write(ServerMessage::StructuredError(ServerError::SessionExpiredRaceTime)).await?;
                            }
                            Err(e) => return Err(e.into()),
                        }
                    }
                    ClientMessage::Stop => if *logged_in_as_admin {
                        //TODO close TCP connections and listener
                        for room in lock!(rooms.0).list.values() {
                            lock!(@write room).save(false).await?;
                        }
                        shutdown.notify();
                        return Err(SessionError::Shutdown)
                    } else {
                        error!("Stop command requires admin login")
                    },
                    ClientMessage::Track { mw_room, tracker_room_name, world_count } => if *logged_in_as_admin {
                        if let Some(mut room) = rooms.write(&mw_room).await {
                            room.init_tracker(tracker_room_name, world_count).await?;
                        } else {
                            error!("no such room")
                        }
                    } else {
                        error!("Track command requires admin login")
                    },
                    ClientMessage::WaitUntilEmpty => if *logged_in_as_admin {
                        waiting_until_empty = true;
                        let mut any_players = false;
                        for room in lock!(rooms.0).list.values() {
                            if lock!(@read room).clients.values().any(|client| client.player.is_some()) {
                                any_players = true;
                                break
                            }
                        }
                        if !any_players {
                            lock!(writer).write(ServerMessage::RoomsEmpty).await?;
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
                    ClientMessage::LeaveRoom => {}
                }
                read = next_message::<C>(reader);
            }
        }
    })
}

async fn update_room_list<C: ClientKind>(rooms: Rooms<C>, writer: Arc<Mutex<C::Writer>>, was_admin: bool, old_mhid: Option<u64>, is_admin: bool, new_mhid: Option<u64>) -> Result<(), SessionError> {
    if was_admin == is_admin && old_mhid == new_mhid { return Ok(()) }
    let lock = lock!(rooms.0);
    for (id, room) in &lock.list {
        let room = lock!(@read room);
        let new_availability = room.auth.availability(is_admin, new_mhid);
        if new_availability != room.auth.availability(was_admin, old_mhid) {
            let password_required = match new_availability {
                RoomAvailability::Open => Some(false),
                RoomAvailability::PasswordRequired => Some(true),
                RoomAvailability::Invisible => None,
            };
            lock!(writer).write(if let Some(password_required) = password_required {
                ServerMessage::NewRoom {
                    id: *id,
                    name: room.name.clone(),
                    password_required,
                }
            } else {
                ServerMessage::DeleteRoom {
                    id: *id,
                    name: room.name.clone(),
                }
            }).await?;
        }
    }
    Ok(())
}

async fn room_session<C: ClientKind>(rooms: Rooms<C>, room: ArcRwLock<Room<C>>, socket_id: C::SessionId, reader: C::Reader, writer: Arc<Mutex<C::Writer>>, maintenance: &mut watch::Receiver<Option<(DateTime<Utc>, Duration)>>, mut end_rx: oneshot::Receiver<EndRoomSession>, mut shutdown: rocket::Shutdown) -> Result<(NextMessage<C>, EndRoomSession), SessionError> {
    macro_rules! error {
        ($($msg:tt)*) => {{
            return Err(SessionError::Server(format!($($msg)*)))
        }};
    }

    let mut read = next_message::<C>(reader);
    Ok(loop {
        select! {
            () = &mut shutdown => return Err(SessionError::Shutdown),
            Ok(()) = maintenance.changed() => {
                let maintenance = *maintenance.borrow_and_update();
                if let Some((start, duration)) = maintenance {
                    lock!(writer).write(ServerMessage::MaintenanceNotice { start, duration }).await?;
                }
            }
            end_res = &mut end_rx => break (read, end_res?),
            res = &mut read => {
                let (reader, msg) = res??;
                match msg {
                    ClientMessage::Ping => {}
                    ClientMessage::JoinRoom { room: Either::Left(id), .. } => if id != lock!(@read room).id {
                        error!("received a JoinRoom message, which only works in the lobby, but you're in a room")
                    },
                    ClientMessage::JoinRoom { room: Either::Right(name), .. } => if name != lock!(@read room).name {
                        error!("received a JoinRoom message, which only works in the lobby, but you're in a room")
                    },
                    ClientMessage::CreateRoom { name, .. } => if name != lock!(@read room).name {
                        error!("received a CreateRoom message, which only works in the lobby, but you're in a room")
                    },
                    ClientMessage::LoginApiKey { .. } => error!("received a LoginApiKey message, which only works in the lobby, but you're in a room"),
                    ClientMessage::LoginDiscord { .. } => error!("received a LoginDiscord message, which only works in the lobby, but you're in a room"),
                    ClientMessage::LoginRaceTime { .. } => error!("received a LoginRaceTime message, which only works in the lobby, but you're in a room"),
                    ClientMessage::Stop => error!("received a Stop message, which only works in the lobby, but you're in a room"),
                    ClientMessage::Track { .. } => error!("received a Track message, which only works in the lobby, but you're in a room"),
                    ClientMessage::WaitUntilEmpty => error!("received a WaitUntilEmpty message, which only works in the lobby, but you're in a room"),
                    ClientMessage::PlayerId(id) => if !lock!(@write room).load_player(socket_id, id).await? {
                        lock!(writer).write(ServerMessage::WorldTaken(id)).await?;
                    },
                    ClientMessage::ResetPlayerId => lock!(@write room).unload_player(socket_id).await?,
                    ClientMessage::PlayerName(name) => lock!(@write room).set_player_name(socket_id, name).await?,
                    ClientMessage::SendItem { key, kind, target_world } => match lock!(@write room).queue_item(socket_id, key, kind, target_world).await {
                        Ok(()) => {}
                        Err(multiworld::QueueItemError::FileHash { server, client }) => lock!(writer).write(ServerMessage::WrongFileHash { server, client }).await?,
                        Err(e) => return Err(e.into()),
                    },
                    ClientMessage::KickPlayer(id) => {
                        let mut room = lock!(@write room);
                        for (&socket_id, client) in &room.clients {
                            if let Some(Player { world, .. }) = client.player {
                                if world == id {
                                    room.remove_client(socket_id, EndRoomSession::ToLobby).await?;
                                    break
                                }
                            }
                        }
                    }
                    ClientMessage::DeleteRoom => {
                        let id = {
                            let mut room = lock!(@write room);
                            room.delete().await?;
                            room.id
                        };
                        rooms.remove(id).await;
                    }
                    ClientMessage::SaveData(save) => lock!(@write room).set_save_data(socket_id, save).await?,
                    ClientMessage::SendAll { source_world, spoiler_log } => match lock!(@write room).send_all(source_world, &spoiler_log).await {
                        Ok(()) => {}
                        Err(multiworld::SendAllError::FileHash { server, client }) => lock!(writer).write(ServerMessage::WrongFileHash { server, client }).await?,
                        Err(e) => return Err(e.into()),
                    },
                    ClientMessage::SaveDataError { debug, version } => if version >= multiworld::version() {
                        eprintln!("save data error reported by Mido's House Multiworld version {version}: {debug}");
                        let _ = Command::new("sudo").arg("-u").arg("fenhl").arg("/opt/night/bin/nightd").arg("report").arg("/games/zelda/oot/mhmw/error").spawn(); //TODO include error details in report
                    },
                    ClientMessage::FileHash(hash) => match lock!(@write room).set_file_hash(socket_id, hash).await {
                        Ok(()) => {}
                        Err(multiworld::SetHashError::FileHash { server, client }) => lock!(writer).write(ServerMessage::WrongFileHash { server, client }).await?,
                        Err(e) => return Err(e.into()),
                    },
                    ClientMessage::AutoDeleteDelta(new_delta) => lock!(@write room).set_autodelete_delta(new_delta).await?,
                    ClientMessage::LeaveRoom => {
                        let mut room = lock!(@write room);
                        room.remove_client(socket_id, EndRoomSession::ToLobby).await?;
                    }
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
    New(ArcRwLock<Room<C>>),
    /// A room has been deleted.
    Delete {
        id: u64,
        name: String,
        auth: RoomAuth,
    },
    /// A player has joined a room.
    Join,
    /// A player has left (or been kicked from) a room.
    Leave,
}

struct RoomsInner<C: ClientKind> {
    list: HashMap<u64, ArcRwLock<Room<C>>>,
    change_tx: broadcast::Sender<RoomListChange<C>>,
    autodelete_tx: broadcast::Sender<(u64, DateTime<Utc>)>,
    #[cfg(unix)]
    inactive_tx: broadcast::Sender<(u64, DateTime<Utc>)>,
}

#[derive(Derivative)]
#[derivative(Clone(bound = ""))]
struct Rooms<C: ClientKind>(Arc<Mutex<RoomsInner<C>>>);

impl<C: ClientKind> Rooms<C> {
    async fn get_arc(&self, room: &Either<u64, String>) -> Option<ArcRwLock<Room<C>>> {
        match room {
            &Either::Left(room_id) => if let Some(room) = lock!(self.0).list.get(&room_id) {
                Some(room.clone())
            } else {
                None
            },
            Either::Right(room_name) => {
                for room in lock!(self.0).list.values() {
                    let room = room.clone();
                    if lock!(@read room).name == *room_name {
                        return Some(room)
                    }
                }
                None
            }
        }
    }

    async fn write(&self, room: &Either<u64, String>) -> Option<OwnedRwLockWriteGuard<Room<C>>> {
        match room {
            &Either::Left(room_id) => if let Some(room) = lock!(self.0).list.get(&room_id) {
                Some(lock!(@write @owned room.clone()))
            } else {
                None
            },
            Either::Right(room_name) => {
                for room in lock!(self.0).list.values() {
                    let room = lock!(@write @owned room.clone());
                    if room.name == *room_name {
                        return Some(room)
                    }
                }
                None
            }
        }
    }

    async fn add(&self, room: ArcRwLock<Room<C>>) -> bool {
        let (id, name) = {
            let room = lock!(@read room);
            (room.id, room.name.clone())
        };
        let mut lock = lock!(self.0);
        for existing_room in lock.list.values() {
            if lock!(@read existing_room).name == name {
                return false
            }
        }
        let hash_map::Entry::Vacant(entry) = lock.list.entry(id) else { return false };
        entry.insert(room.clone());
        let _ = lock.change_tx.send(RoomListChange::New(room));
        true
    }

    async fn remove(&self, id: u64) {
        let mut lock = lock!(self.0);
        if let Some(room) = lock.list.remove(&id) {
            let room = lock!(@read room);
            let name = room.name.clone();
            let auth = room.auth.clone();
            let _ = lock.change_tx.send(RoomListChange::Delete { id, name, auth });
        }
    }

    async fn wait_cleanup(&self, mut shutdown: rocket::Shutdown) -> Result<(), broadcast::error::RecvError> {
        let (mut autodelete_at, mut autodelete_rx) = {
            let lock = lock!(self.0);
            (
                stream::iter(&lock.list).then(|(name, room)| async move { (name.clone(), lock!(@read room).autodelete_at()) }).collect::<HashMap<_, _>>().await,
                lock.autodelete_tx.subscribe(),
            )
        };
        Ok(loop {
            let now = Utc::now();
            let sleep = if let Some(&time) = autodelete_at.values().min() {
                EitherFuture::Left(if let Ok(delta) = (time - now).to_std() {
                    EitherFuture::Left(sleep(delta))
                } else {
                    // target time is in the past
                    EitherFuture::Right(future::ready(()))
                })
            } else {
                EitherFuture::Right(future::pending())
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
            let lock = lock!(self.0);
            (
                stream::iter(&lock.list).then(|(name, room)| async move { (name.clone(), lock!(@read room).last_saved + chrono::Duration::hours(1)) }).collect::<HashMap<_, _>>().await,
                lock.inactive_tx.subscribe(),
            )
        };
        Ok(loop {
            let now = Utc::now();
            let sleep = if let Some(&time) = inactive_at.values().min() {
                EitherFuture::Left(if let Ok(delta) = (time - now).to_std() {
                    EitherFuture::Left(sleep(delta))
                } else {
                    // target time is in the past
                    EitherFuture::Right(future::ready(()))
                })
            } else {
                EitherFuture::Right(future::pending())
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
    #[clap(short, long, default_value = "midos_house")]
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
    #[error(transparent)] Reqwest(#[from] reqwest::Error),
    #[error(transparent)] Ring(#[from] ring::error::Unspecified),
    #[error(transparent)] Rocket(#[from] rocket::Error),
    #[error(transparent)] Sql(#[from] sqlx::Error),
    #[error(transparent)] Task(#[from] tokio::task::JoinError),
    #[cfg(unix)] #[error(transparent)] Wheel(#[from] wheel::Error),
    #[error(transparent)] Write(#[from] async_proto::WriteError),
    #[cfg(unix)]
    #[error("error while creating tournament room")]
    CreateTournamentRoom,
    #[cfg(unix)]
    #[error("error while waiting until inactive")]
    WaitUntilInactive,
}

#[wheel::main(rocket)]
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
                        WaitUntilInactiveMessage::Deadline(_) => unreachable!(),
                    }
                },
                Subcommand::PrepareRestart => {
                    let mut deadline = None::<DateTime<Utc>>;
                    loop {
                        match WaitUntilInactiveMessage::read(&mut sock).await? {
                            WaitUntilInactiveMessage::Error => return Err(Error::WaitUntilInactive),
                            WaitUntilInactiveMessage::ActiveRooms(rooms) => if let Some(deadline) = deadline {
                                wheel::print_flush!(
                                    "\r[....] waiting for {} rooms to be inactive (current ETA: {}) or until {} ",
                                    rooms.len(),
                                    rooms.values().map(|(inactive_at, _)| inactive_at).max().expect("waiting for 0 rooms").format("%Y-%m-%d %H:%M:%S UTC"),
                                    deadline.format("%Y-%m-%d %H:%M:%S UTC"),
                                )?;
                            } else {
                                wheel::print_flush!(
                                    "\r[....] waiting for {} rooms to be inactive (current ETA: {}) ",
                                    rooms.len(),
                                    rooms.values().map(|(inactive_at, _)| inactive_at).max().expect("waiting for 0 rooms").format("%Y-%m-%d %H:%M:%S UTC"),
                                )?;
                            },
                            WaitUntilInactiveMessage::Inactive => {
                                println!("[ ok ]");
                                break
                            }
                            WaitUntilInactiveMessage::Deadline(new_deadline) => deadline = Some(new_deadline),
                        }
                    }
                },
                Subcommand::CreateTournamentRoom { .. } => if !bool::read(&mut sock).await? {
                    return Err(Error::CreateTournamentRoom)
                },
            }
            return Ok(())
        }
        #[cfg(not(unix))] match subcommand {}
    } else {
        let default_panic_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = Command::new("sudo").arg("-u").arg("fenhl").arg("/opt/night/bin/nightd").arg("report").arg("/games/zelda/oot/mhmw/error").spawn(); //TODO include error details in report
            default_panic_hook(info)
        }));
        let rng = Arc::new(SystemRandom::new());
        let http_client = reqwest::Client::builder()
            .user_agent(concat!("MidosHouse/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(30))
            .use_rustls_tls()
            .trust_dns(true)
            .https_only(true)
            .build()?;
        let db_pool = PgPool::connect_with(PgConnectOptions::default().username("mido").database(&database).application_name("ootrmwd")).await?;
        let maintenance = Arc::new(watch::channel(None).0);
        let rooms = Rooms::default();
        {
            let mut query = sqlx::query!(r#"SELECT
                id,
                name,
                password_hash AS "password_hash: [u8; CREDENTIAL_LEN]",
                password_salt AS "password_salt: [u8; CREDENTIAL_LEN]",
                invites,
                base_queue,
                player_queues,
                last_saved,
                allow_send_all,
                autodelete_delta
            FROM mw_rooms"#).fetch(&db_pool);
            while let Some(row) = query.try_next().await? {
                assert!(rooms.add(ArcRwLock::new(Room {
                    id: row.id as u64,
                    name: row.name.clone(),
                    auth: match (row.password_hash, row.password_salt) {
                        (Some(hash), Some(salt)) => RoomAuth::Password { hash, salt },
                        (None, None) => RoomAuth::Invitational(Vec::read_sync(&mut &*row.invites)?),
                        (_, _) => unimplemented!(), //TODO add constraint to table
                    },
                    clients: HashMap::default(),
                    file_hash: None, //TODO store in database
                    base_queue: Vec::read_sync(&mut &*row.base_queue)?,
                    player_queues: HashMap::read_sync(&mut &*row.player_queues)?,
                    last_saved: row.last_saved,
                    allow_send_all: row.allow_send_all,
                    autodelete_delta: decode_pginterval(row.autodelete_delta)?,
                    autodelete_tx: {
                        let rooms = lock!(rooms.0);
                        rooms.autodelete_tx.clone()
                    },
                    db_pool: db_pool.clone(),
                    tracker_state: None,
                })).await);
            }
        }
        let rocket = http::rocket(db_pool.clone(), http_client, rng.clone(), port, rooms.clone(), maintenance.clone()).await?;
        #[cfg(unix)] let unix_socket_task = tokio::spawn(unix_socket::listen(db_pool.clone(), rooms.clone(), rocket.shutdown(), maintenance)).map(|res| match res {
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
                    let rooms = lock!(rooms.0);
                    let mut rooms_to_delete = pin!(stream::iter(rooms.list.values()).filter(|room| async { lock!(@read room).autodelete_at() <= now }));
                    rooms_to_delete.next().await.cloned()
                } {
                    let id = {
                        let mut room = lock!(@write room);
                        room.delete().await?;
                        room.id
                    };
                    rooms.remove(id).await;
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
