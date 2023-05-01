use {
    std::{
        collections::HashMap,
        mem,
    },
    async_proto::{
        Protocol,
        ReadError,
    },
    chrono::prelude::*,
    tokio::{
        io,
        net::UnixListener,
        select,
        sync::broadcast,
    },
    wheel::{
        fs,
        traits::IoResultExt as _,
    },
    multiworld::ClientKind,
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
}

#[derive(Protocol)]
pub(crate) enum WaitUntilInactiveMessage {
    Error,
    ActiveRooms(HashMap<String, (DateTime<Utc>, u64)>),
    Inactive,
}

pub(crate) async fn listen<C: ClientKind + 'static>(mut shutdown: rocket::Shutdown, rooms: Rooms<C>) -> wheel::Result<()> {
    fs::remove_file(PATH).await.missing_ok()?;
    let listener = UnixListener::bind(PATH).at(PATH)?;
    loop {
        select! {
            () = &mut shutdown => break,
            res = listener.accept() => {
                let (mut sock, _) = res.at_unknown()?;
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
                                        Ok(RoomListChange::Delete(_)) => {}
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
                                let now = Utc::now();
                                let previous_active_rooms = mem::take(&mut active_rooms);
                                for (room_name, room) in &rooms.0.lock().await.list {
                                    let room = room.read().await;
                                    if room.last_saved > now - chrono::Duration::hours(1) && room.clients.values().any(|client| client.player.is_some()) {
                                        active_rooms.insert(room_name.clone(), (room.last_saved, room.clients.values().filter(|client| client.player.is_some()).count().try_into().expect("too many players")));
                                    }
                                }
                                if active_rooms.is_empty() { break }
                                if active_rooms != previous_active_rooms {
                                    WaitUntilInactiveMessage::ActiveRooms(active_rooms.clone()).write(&mut sock).await.expect("error writing to UNIX socket");
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
