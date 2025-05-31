use {
    std::{
        collections::HashMap,
        mem,
        num::NonZero,
        sync::Arc,
        time::Duration,
    },
    async_proto::{
        Protocol,
        ReadError,
        ReadErrorKind,
    },
    chrono::{
        prelude::*,
        TimeDelta,
    },
    either::Either,
    futures::future,
    log_lock::{
        ArcRwLock,
        lock,
    },
    ootr_utils::spoiler::HashIcon,
    rand::{
        prelude::*,
        rng,
    },
    sqlx::PgPool,
    tokio::{
        io,
        net::UnixListener,
        select,
        sync::{
            broadcast,
            watch,
        },
        time::sleep,
    },
    wheel::{
        fs,
        traits::IoResultExt as _,
    },
    multiworld::{
        ClientKind,
        Room,
        RoomAuth,
        RoomMetadata,
        WaitUntilInactiveMessage,
    },
    crate::{
        RoomListChange,
        Rooms,
    },
};

pub(crate) const PATH: &str = "/usr/local/share/midos-house/sock-mw"; //TODO different path for dev env

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
        #[clap(long)]
        tracker_room_name: Option<String>,
    },
    PrepareRestart {
        #[clap(long)]
        async_proto: bool,
    },
}

pub(crate) async fn listen<C: ClientKind + 'static>(db_pool: PgPool, rooms: Rooms<C>, mut shutdown: rocket::Shutdown, maintenance: Arc<watch::Sender<Option<(DateTime<Utc>, Duration)>>>) -> wheel::Result<()> {
    fs::remove_file(PATH).await.missing_ok()?;
    let listener = UnixListener::bind(PATH).at(PATH)?;
    loop {
        select! {
            () = &mut shutdown => break,
            res = listener.accept() => {
                let (mut sock, _) = res.at_unknown()?;
                let db_pool = db_pool.clone();
                let rooms = rooms.clone();
                let mut shutdown = shutdown.clone();
                let maintenance = maintenance.clone();
                tokio::spawn(async move {
                    let msg = match ClientMessage::read(&mut sock).await {
                        Ok(msg) => msg,
                        Err(ReadError { kind: ReadErrorKind::Io(e), .. }) if e.kind() == io::ErrorKind::UnexpectedEof => return,
                        Err(e) => panic!("error reading from UNIX socket: {e} ({e:?})"),
                    };
                    match msg {
                        ClientMessage::Stop | ClientMessage::StopWhenEmpty | ClientMessage::WaitUntilEmpty => {
                            if let ClientMessage::StopWhenEmpty | ClientMessage::WaitUntilEmpty = msg {
                                let mut room_stream = lock!(rooms = rooms.0; rooms.change_tx.subscribe());
                                loop {
                                    match room_stream.recv().await {
                                        Ok(RoomListChange::New(_)) => {}
                                        Ok(RoomListChange::Delete { .. }) => {}
                                        Ok(RoomListChange::Join | RoomListChange::Leave) => {}
                                        Err(broadcast::error::RecvError::Closed) => unreachable!("room list should be maintained indefinitely"),
                                        Err(broadcast::error::RecvError::Lagged(_)) => room_stream = lock!(rooms = rooms.0; rooms.change_tx.subscribe()),
                                    }
                                    let mut any_players = false;
                                    lock!(rooms = rooms.0; for room in rooms.list.values() {
                                        if lock!(@read room = room; room.clients.values().any(|client| client.player.is_some())) {
                                            any_players = true;
                                            break
                                        }
                                    });
                                    if !any_players { break }
                                }
                            }
                            if let ClientMessage::Stop | ClientMessage::StopWhenEmpty = msg {
                                lock!(rooms = rooms.0; for room in rooms.list.values() {
                                    let _ = lock!(@write room = room; room.save(false).await);
                                });
                                shutdown.notify();
                                0u8.write(&mut sock).await.expect("error writing to UNIX socket");
                                return
                            }
                            0u8.write(&mut sock).await.expect("error writing to UNIX socket");
                        }
                        ClientMessage::WaitUntilInactive => {
                            let mut active_rooms = HashMap::default();
                            let mut room_stream = lock!(rooms = rooms.0; rooms.change_tx.subscribe());
                            loop {
                                let now = Utc::now();
                                let previous_active_rooms = mem::take(&mut active_rooms);
                                lock!(rooms = rooms.0; for room in rooms.list.values() {
                                    lock!(@read room = room; if room.last_saved > now - TimeDelta::try_hours(1).expect("1-hour timedelta out of bounds") && room.clients.values().any(|client| client.player.is_some()) {
                                        active_rooms.insert(room.name.clone(), (room.last_saved + TimeDelta::try_hours(1).expect("1-hour timedelta out of bounds"), room.clients.values().filter(|client| client.player.is_some()).count().try_into().expect("too many players")));
                                    });
                                });
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
                                    () = &mut shutdown => break,
                                }
                            }
                            WaitUntilInactiveMessage::Inactive.write(&mut sock).await.expect("error writing to UNIX socket");
                            return
                        }
                        ClientMessage::CreateTournamentRoom { name, hash1, hash2, hash3, hash4, hash5, players, tracker_room_name } => {
                            let id = loop {
                                let id = rng().random::<u64>();
                                match sqlx::query_scalar!(r#"SELECT EXISTS (SELECT 1 FROM mw_rooms WHERE id = $1) AS "exists!""#, id as i64).fetch_one(&db_pool).await {
                                    Ok(true) => {}
                                    Ok(false) => break id, //TODO save room to database in same transaction
                                    Err(_) => {
                                        false.write(&mut sock).await.expect("error writing to UNIX socket");
                                        return
                                    }
                                }
                            };
                            let world_count = players.len().try_into().ok().and_then(NonZero::new);
                            let now = Utc::now();
                            let room = ArcRwLock::new(Room {
                                auth: RoomAuth::Invitational(players),
                                clients: HashMap::default(),
                                file_hash: Some(Some([hash1, hash2, hash3, hash4, hash5])),
                                base_queue: Vec::default(),
                                player_queues: HashMap::default(),
                                last_saved: now,
                                deleted: false,
                                created: Some(now),
                                allow_send_all: false,
                                autodelete_delta: Duration::from_secs(60 * 60 * 24),
                                autodelete_tx: lock!(rooms = rooms.0; rooms.autodelete_tx.clone()),
                                db_pool: db_pool.clone(),
                                tracker_state: None,
                                metadata: RoomMetadata::default(),
                                id, name,
                            });
                            (rooms.add(room.clone()).await.is_ok()
                            && if let Some(tracker_room_name) = tracker_room_name {
                                if let Some(world_count) = world_count {
                                    lock!(@write room = room; room.init_tracker(tracker_room_name, world_count).await).is_ok()
                                } else {
                                    false
                                }
                            } else {
                                true
                            }).write(&mut sock).await.expect("error writing to UNIX socket");
                        }
                        ClientMessage::PrepareRestart { async_proto: _ } => {
                            let mut deadline = Utc::now() + TimeDelta::try_days(1).expect("1-day timedelta out of bounds");
                            loop {
                                match sqlx::query_scalar!(r#"SELECT start AS "start!" FROM races WHERE series = 'mw' AND start > $1::TIMESTAMPTZ - INTERVAL '24:00:00' AND start <= $1::TIMESTAMPTZ + INTERVAL '00:15:00' ORDER BY start DESC LIMIT 1"#, deadline).fetch_optional(&db_pool).await {
                                    Ok(Some(start)) => {
                                        deadline = start + TimeDelta::try_days(1).expect("1-day timedelta out of bounds");
                                        continue
                                    }
                                    Ok(None) => {}
                                    Err(_) => {
                                        WaitUntilInactiveMessage::Error.write(&mut sock).await.expect("error writing to UNIX socket");
                                        return
                                    }
                                }
                                match sqlx::query_scalar!(r#"SELECT async_start1 AS "async_start1!" FROM races WHERE series = 'mw' AND async_start1 > $1::TIMESTAMPTZ - INTERVAL '24:00:00' AND async_start1 <= $1::TIMESTAMPTZ + INTERVAL '00:15:00' ORDER BY async_start1 DESC LIMIT 1"#, deadline).fetch_optional(&db_pool).await {
                                    Ok(Some(start)) => {
                                        deadline = start + TimeDelta::try_days(1).expect("1-day timedelta out of bounds");
                                        continue
                                    }
                                    Ok(None) => {}
                                    Err(_) => {
                                        WaitUntilInactiveMessage::Error.write(&mut sock).await.expect("error writing to UNIX socket");
                                        return
                                    }
                                }
                                match sqlx::query_scalar!(r#"SELECT async_start2 AS "async_start2!" FROM races WHERE series = 'mw' AND async_start2 > $1::TIMESTAMPTZ - INTERVAL '24:00:00' AND async_start2 <= $1::TIMESTAMPTZ + INTERVAL '00:15:00' ORDER BY async_start2 DESC LIMIT 1"#, deadline).fetch_optional(&db_pool).await {
                                    Ok(Some(start)) => {
                                        deadline = start + TimeDelta::try_days(1).expect("1-day timedelta out of bounds");
                                        continue
                                    }
                                    Ok(None) => {}
                                    Err(_) => {
                                        WaitUntilInactiveMessage::Error.write(&mut sock).await.expect("error writing to UNIX socket");
                                        return
                                    }
                                }
                                break
                            }
                            //TODO adjust deadline for races scheduled during the wait
                            WaitUntilInactiveMessage::Deadline(deadline).write(&mut sock).await.expect("error writing to UNIX socket");
                            maintenance.send_replace(Some((deadline, Duration::from_secs(5 * 60)))); //TODO measure actual downtime duration and use as estimate
                            let mut active_rooms = HashMap::default();
                            let mut room_stream = lock!(rooms = rooms.0; rooms.change_tx.subscribe());
                            loop {
                                let now = Utc::now();
                                let previous_active_rooms = mem::take(&mut active_rooms);
                                lock!(rooms = rooms.0; for room in rooms.list.values() {
                                    lock!(@read room = room; if room.last_saved > now - TimeDelta::try_hours(1).expect("1-hour timedelta out of bounds") && room.clients.values().any(|client| client.player.is_some()) {
                                        active_rooms.insert(room.name.clone(), (room.last_saved + TimeDelta::try_hours(1).expect("1-hour timedelta out of bounds"), room.clients.values().filter(|client| client.player.is_some()).count().try_into().expect("too many players")));
                                    });
                                });
                                if active_rooms.is_empty() { break }
                                if active_rooms != previous_active_rooms {
                                    WaitUntilInactiveMessage::ActiveRooms(active_rooms.clone()).write(&mut sock).await.expect("error writing to UNIX socket");
                                }
                                let sleep = if let Ok(duration) = (deadline - Utc::now()).to_std() {
                                    Either::Left(sleep(duration))
                                } else {
                                    Either::Right(future::ready(()))
                                };
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
                                    () = sleep => break,
                                    () = &mut shutdown => break,
                                }
                            }
                            WaitUntilInactiveMessage::Inactive.write(&mut sock).await.expect("error writing to UNIX socket");
                            return
                        }
                    }
                });
            }
        }
    }
    Ok(())
}
