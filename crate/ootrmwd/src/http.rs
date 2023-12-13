use {
    std::{
        sync::{
            Arc,
            atomic::{
                AtomicUsize,
                Ordering::*,
            },
        },
        time::Duration,
    },
    chrono::prelude::*,
    futures::stream::StreamExt as _,
    log_lock::*,
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
        process::Command,
        sync::watch,
    },
    tokio_tungstenite::tungstenite,
    multiworld::{
        ClientWriter as _,
        ws::{
            Version,
            VersionedReader,
            VersionedWriter,
            unversioned::ServerMessage,
        },
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

macro_rules! supported_version {
    ($endpoint:literal, $version:ident, $variant:ident, $number:literal) => {
        #[rocket::get($endpoint)]
        async fn $version(rng: &State<Arc<SystemRandom>>, db_pool: &State<PgPool>, http_client: &State<reqwest::Client>, rooms: &State<Rooms<WebSocket>>, maintenance: &State<Arc<watch::Sender<Option<(DateTime<Utc>, Duration)>>>>, next_session_id: &State<AtomicUsize>, ws: WebSocket, shutdown: rocket::Shutdown) -> rocket_ws::Channel<'static> {
            let _ = sqlx::query!("INSERT INTO mw_versions (version, last_used) VALUES ($1, NOW()) ON CONFLICT (version) DO UPDATE SET last_used = EXCLUDED.last_used", $number).execute(&**db_pool).await;
            let rng = (*rng).clone();
            let db_pool = (*db_pool).clone();
            let http_client = (*http_client).clone();
            let rooms = (*rooms).clone();
            let maintenance = (*maintenance).clone();
            let session_id = next_session_id.fetch_add(1, SeqCst);
            ws.channel(move |stream| Box::pin(async move {
                let (sink, stream) = stream.split();
                let writer = Arc::new(Mutex::new(VersionedWriter { inner: sink, version: Version::$variant }));
                match client_session(&rng, db_pool.clone(), http_client, rooms, session_id, VersionedReader { inner: stream, version: Version::$variant }, Arc::clone(&writer), shutdown, maintenance).await {
                    Ok(()) => {}
                    Err(SessionError::Read(async_proto::ReadError { kind: async_proto::ReadErrorKind::MessageKind(tungstenite::Message::Close(_)), .. })) => {} // client disconnected normally
                    Err(SessionError::Read(async_proto::ReadError { kind: async_proto::ReadErrorKind::Tungstenite(tungstenite::Error::Protocol(tungstenite::error::ProtocolError::ResetWithoutClosingHandshake)), .. })) => {} // this happens when a player force quits their multiworld app (or normally quits on macOS, see https://github.com/iced-rs/iced/issues/1941)
                    Err(SessionError::Server(msg)) => {
                        eprintln!("server error in WebSocket handler ({}): {msg}", stringify!($version));
                        let _ = Command::new("sudo").arg("-u").arg("fenhl").arg("/opt/night/bin/nightd").arg("report").arg("/games/zelda/oot/mhmw/error").spawn(); //TODO include error details in report
                        let _ = lock!(writer).write(ServerMessage::OtherError(msg)).await;
                    }
                    Err(e) => {
                        eprintln!("error in WebSocket handler ({}): {e}", stringify!($version));
                        eprintln!("debug info: {e:?}");
                        let _ = Command::new("sudo").arg("-u").arg("fenhl").arg("/opt/night/bin/nightd").arg("report").arg("/games/zelda/oot/mhmw/error").spawn(); //TODO include error details in report
                        let _ = lock!(writer).write(ServerMessage::OtherError(e.to_string())).await;
                    }
                }
                let _ = sqlx::query!("INSERT INTO mw_versions (version, last_used) VALUES ($1, NOW()) ON CONFLICT (version) DO UPDATE SET last_used = EXCLUDED.last_used", $number).execute(&db_pool).await;
                Ok(())
            }))
        }
    };
}

supported_version!("/v10", v10, V10, 10);
supported_version!("/v11", v11, V11, 11);
supported_version!("/v12", v12, V12, 12);
supported_version!("/v13", v13, V13, 13);
supported_version!("/v14", v14, V14, 14);

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

pub(crate) async fn rocket(db_pool: PgPool, http_client: reqwest::Client, rng: Arc<SystemRandom>, port: u16, rooms: Rooms<WebSocket>, maintenance: Arc<watch::Sender<Option<(DateTime<Utc>, Duration)>>>) -> Result<Rocket<rocket::Ignite>, crate::Error> {
    Ok(rocket::custom(rocket::Config {
        log_level: rocket::config::LogLevel::Critical,
        port,
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
    .manage(http_client)
    .manage(rng)
    .manage(rooms)
    .manage(maintenance)
    .manage(AtomicUsize::default())
    .ignite().await?)
}
