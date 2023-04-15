use {
    std::sync::{
        Arc,
        atomic::{
            AtomicUsize,
            Ordering::*,
        },
    },
    futures::stream::StreamExt as _,
    ring::rand::SystemRandom,
    rocket::{
        Rocket,
        State,
        response::{
            Redirect,
            content::RawHtml,
        },
        uri,
    },
    rocket_util::{
        Doctype,
        html,
    },
    rocket_ws::WebSocket,
    sqlx::PgPool,
    tokio::{
        io,
        sync::Mutex,
    },
    crate::{
        Rooms,
        SessionError,
        client_session,
    },
};

#[rocket::get("/")]
fn index() -> Redirect {
    Redirect::permanent(uri!("https://midos.house/mw"))
}

#[rocket::get("/v10")]
fn v10(rng: &State<Arc<SystemRandom>>, db_pool: &State<PgPool>, rooms: &State<Rooms<WebSocket>>, next_session_id: &State<AtomicUsize>, ws: WebSocket, shutdown: rocket::Shutdown) -> rocket_ws::Channel<'static> {
    let rng = (*rng).clone();
    let db_pool = (*db_pool).clone();
    let rooms = (*rooms).clone();
    let session_id = next_session_id.fetch_add(1, SeqCst);
    ws.channel(move |stream| Box::pin(async move {
        let (sink, stream) = stream.split();
        client_session(&rng, db_pool, rooms, session_id, stream, Arc::new(Mutex::new(sink)), shutdown).await.map_err(|e| match e {
            SessionError::Elapsed(e) => tungstenite::Error::Io(io::Error::new(io::ErrorKind::TimedOut, e)),
            SessionError::Read(e) => tungstenite::Error::Io(e.into()),
            SessionError::Shutdown => tungstenite::Error::Io(io::Error::new(io::ErrorKind::ConnectionAborted, e)),
            SessionError::Write(e) => tungstenite::Error::Io(e.into()),
            SessionError::OneshotRecv(_) |
            SessionError::QueueItem(_) |
            SessionError::Ring(_) |
            SessionError::SendAll(_) |
            SessionError::SetHash(_) |
            SessionError::Sql(_) |
            SessionError::Server(_) => tungstenite::Error::Io(io::Error::new(io::ErrorKind::Other, e)),
        })
    }))
}

#[rocket::catch(404)]
async fn not_found() -> RawHtml<String> {
    html! {
        : Doctype;
        html {
            head {
                meta(charset = "utf-8");
                meta(name = "viewport", content = "width=device-width, initial-scale=1, shrink-to-fit=no");
                title : "Not Found — Mido's House Multiworld";
                link(rel = "icon", sizes = "1024x1024", type = "image/png", href = uri!("https://midos.house/static/mw.png").to_string());
                link(rel = "stylesheet", href = uri!("https://midos.house/static/common.css").to_string());
            }
            body {
                h1 : "Error 404: Not Found";
                p : "There is no page at this address.";
            }
        }
    }
}

#[rocket::catch(500)]
async fn internal_server_error() -> RawHtml<String> {
    html! {
        : Doctype;
        html {
            head {
                meta(charset = "utf-8");
                meta(name = "viewport", content = "width=device-width, initial-scale=1, shrink-to-fit=no");
                title : "Internal Server Error — Mido's House Multiworld";
                link(rel = "icon", sizes = "1024x1024", type = "image/png", href = uri!("https://midos.house/static/mw.png").to_string());
                link(rel = "stylesheet", href = uri!("https://midos.house/static/common.css").to_string());
            }
            body {
                h1 : "Error 500: Internal Server Error";
                p : "Sorry, something went wrong. Please notify Fenhl on Discord.";
            }
        }
    }
}

pub(crate) async fn rocket(db_pool: PgPool, rng: Arc<SystemRandom>, rooms: Rooms<WebSocket>) -> Result<Rocket<rocket::Ignite>, crate::Error> {
    Ok(rocket::custom(rocket::Config {
        log_level: rocket::config::LogLevel::Critical,
        port: 24819,
        ..rocket::Config::default()
    })
    .mount("/", multiworld_derive::routes![
        index,
        // WebSocket routes added automatically
    ])
    .register("/", rocket::catchers![
        not_found,
        internal_server_error,
    ])
    .manage(db_pool)
    .manage(rng)
    .manage(rooms)
    .manage(Arc::<AtomicUsize>::default())
    .ignite().await?)
}
