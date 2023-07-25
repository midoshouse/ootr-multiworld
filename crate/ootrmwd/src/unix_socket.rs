use {
    std::{
        collections::HashMap,
        mem,
        sync::Arc,
        time::Duration,
    },
    async_proto::{
        Protocol,
        ReadError,
    },
    chrono::prelude::*,
    ootr_utils::spoiler::HashIcon,
    rand::prelude::*,
    sqlx::PgPool,
    tokio::{
        io,
        net::UnixListener,
        select,
        sync::{
            RwLock,
            broadcast,
        },
    },
    wheel::{
        fs,
        traits::IoResultExt as _,
    },
    multiworld::{
        ClientKind,
        Room,
        RoomAuth,
    },
    crate::{
        RoomListChange,
        Rooms,
    },
};

pub(crate) const PATH: &str = "/usr/local/share/midos-house/sock-mw";

#[derive(clap::Subcommand, Protocol)]
pub(crate) enum ClientMessage {
    Stop,
    StopWhenEmpty,
    WaitUntilEmpty,
    WaitUntilInactive,
    CreateTournamentRoom {
        name: String,
        hash1: HashIcon,
        hash2: HashIcon,
        hash3: HashIcon,
        hash4: HashIcon,
        hash5: HashIcon,
        players: Vec<u64>,
    },
}

#[derive(Protocol)]
pub(crate) enum WaitUntilInactiveMessage {
    Error,
    ActiveRooms(HashMap<String, (DateTime<Utc>, u64)>),
    Inactive,
}

pub(crate) async fn listen<C: ClientKind + 'static>(db_pool: PgPool, rooms: Rooms<C>, mut shutdown: rocket::Shutdown) -> wheel::Result<()> {
    fs::remove_file(PATH).await.missing_ok()?;
    let listener = UnixListener::bind(PATH).at(PATH)?;
    loop {
        select! {
            () = &mut shutdown => break,
            res = listener.accept() => {
                let (mut sock, _) = res.at_unknown()?;
                let db_pool = db_pool.clone();
                let rooms = rooms.clone();
                let shutdown = shutdown.clone();
                tokio::spawn(async move {
                    let msg = match ClientMessage::read(&mut sock).await {
                        Ok(msg) => msg,
                        Err(ReadError::Io(e)) if e.kind() == io::ErrorKind::UnexpectedEof => return,
                        Err(e) => panic!("error reading from UNIX socket: {e} ({e:?})"),
                    };
                    match msg {
                        ClientMessage::Stop | ClientMessage::StopWhenEmpty | ClientMessage::WaitUntilEmpty => {
                            if let ClientMessage::StopWhenEmpty | ClientMessage::WaitUntilEmpty = msg {
                                let mut room_stream = rooms.0.lock().await.change_tx.subscribe();
                                loop {
                                    match room_stream.recv().await {
                                        Ok(RoomListChange::New(_)) => {}
                                        Ok(RoomListChange::Delete { .. }) => {}
                                        Ok(RoomListChange::Join | RoomListChange::Leave) => {}
                                        Err(broadcast::error::RecvError::Closed) => unreachable!("room list should be maintained indefinitely"),
                                        Err(broadcast::error::RecvError::Lagged(_)) => room_stream = rooms.0.lock().await.change_tx.subscribe(),
                                    }
                                    let mut any_players = false;
                                    for room in rooms.0.lock().await.list.values() {
                                        if room.read().await.clients.values().any(|client| client.player.is_some()) {
                                            any_players = true;
                                            break
                                        }
                                    }
                                    if !any_players { break }
                                }
                            }
                            if let ClientMessage::Stop | ClientMessage::StopWhenEmpty = msg {
                                for room in rooms.0.lock().await.list.values() {
                                    let _ = room.write().await.save(false).await;
                                }
                                shutdown.notify();
                                0u8.write(&mut sock).await.expect("error writing to UNIX socket");
                                return
                            }
                            0u8.write(&mut sock).await.expect("error writing to UNIX socket");
                        }
                        ClientMessage::WaitUntilInactive => { //TODO use this instead of wait-until-empty in release script after the next release after 10.0.3
                            let mut active_rooms = HashMap::default();
                            let mut room_stream = rooms.0.lock().await.change_tx.subscribe();
                            loop {
                                let now = Utc::now();
                                let previous_active_rooms = mem::take(&mut active_rooms);
                                for room in rooms.0.lock().await.list.values() {
                                    let room = room.read().await;
                                    if room.last_saved > now - chrono::Duration::hours(1) && room.clients.values().any(|client| client.player.is_some()) {
                                        active_rooms.insert(room.name.clone(), (room.last_saved + chrono::Duration::hours(1), room.clients.values().filter(|client| client.player.is_some()).count().try_into().expect("too many players")));
                                    }
                                }
                                if active_rooms.is_empty() { break }
                                if active_rooms != previous_active_rooms {
                                    WaitUntilInactiveMessage::ActiveRooms(active_rooms.clone()).write(&mut sock).await.expect("error writing to UNIX socket");
                                }
                                select! {
                                    res = rooms.wait_inactive(shutdown.clone()) => match res {
                                        Ok(()) => {}
                                        Err(_) => {
                                            WaitUntilInactiveMessage::Error.write(&mut sock).await.expect("error writing to UNIX socket");
                                            return
                                        }
                                    },
                                    res = room_stream.recv() => match res {
                                        Ok(_) => {}
                                        Err(_) => {
                                            WaitUntilInactiveMessage::Error.write(&mut sock).await.expect("error writing to UNIX socket");
                                            return
                                        }
                                    },
                                }
                            }
                            WaitUntilInactiveMessage::Inactive.write(&mut sock).await.expect("error writing to UNIX socket");
                            return
                        }
                        ClientMessage::CreateTournamentRoom { name, hash1, hash2, hash3, hash4, hash5, players } => {
                            let id = loop {
                                let id = thread_rng().gen::<u64>();
                                match sqlx::query_scalar!(r#"SELECT EXISTS (SELECT 1 FROM mw_rooms WHERE id = $1) AS "exists!""#, id as i64).fetch_one(&db_pool).await {
                                    Ok(true) => {}
                                    Ok(false) => break id, //TODO save room to database in same transaction
                                    Err(_) => {
                                        false.write(&mut sock).await.expect("error writing to UNIX socket");
                                        return
                                    }
                                }
                            };
                            rooms.add(Arc::new(RwLock::new(Room {
                                auth: RoomAuth::Invitational(players),
                                clients: HashMap::default(),
                                file_hash: Some([hash1, hash2, hash3, hash4, hash5]),
                                base_queue: Vec::default(),
                                player_queues: HashMap::default(),
                                last_saved: Utc::now(),
                                allow_send_all: false,
                                autodelete_delta: Duration::from_secs(60 * 60 * 24),
                                autodelete_tx: {
                                    let rooms = rooms.0.lock().await;
                                    rooms.autodelete_tx.clone()
                                },
                                db_pool: db_pool.clone(),
                                tracker_state: None,
                                id, name,
                            }))).await.write(&mut sock).await.expect("error writing to UNIX socket");
                        }
                    }
                });
            }
        }
    }
    Ok(())
}
