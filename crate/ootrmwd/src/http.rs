use {
    std::sync::{
        Arc,
        atomic::{
            AtomicUsize,
            Ordering::*,
        },
    },
    futures::stream::StreamExt as _,
    log_lock::Mutex,
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
    multiworld::ws::{
        Version,
        VersionedReader,
        VersionedWriter,
    },
    crate::{
        Rooms,
        client_session,
    },
};

#[rocket::get("/")]
fn index() -> Redirect {
    Redirect::permanent(uri!("https://midos.house/mw"))
}

macro_rules! supported_version {
    ($endpoint:literal, $version:ident, $variant:ident) => {
        #[rocket::get($endpoint)]
        fn $version(rng: &State<Arc<SystemRandom>>, db_pool: &State<PgPool>, http_client: &State<reqwest::Client>, rooms: &State<Rooms<WebSocket>>, next_session_id: &State<AtomicUsize>, ws: WebSocket, shutdown: rocket::Shutdown) -> rocket_ws::Channel<'static> {
            let rng = (*rng).clone();
            let db_pool = (*db_pool).clone();
            let http_client = (*http_client).clone();
            let rooms = (*rooms).clone();
            let session_id = next_session_id.fetch_add(1, SeqCst);
            ws.channel(move |stream| Box::pin(async move {
                let (sink, stream) = stream.split();
                match client_session(&rng, db_pool, http_client, rooms, session_id, VersionedReader { inner: stream, version: Version::$variant }, Arc::new(Mutex::new(VersionedWriter { inner: sink, version: Version::$variant })), shutdown).await {
                    Ok(()) => {}
                    Err(e) => {
                        eprintln!("error in WebSocket handler ({}): {e}", stringify!($version));
                        eprintln!("debug info: {e:?}");
                        //TODO send error to client? (currently handled individually for each error)
                    }
                }
                Ok(())
            }))
        }
    };
}

supported_version!("/v10", v10, V10);
supported_version!("/v11", v11, V11);
supported_version!("/v12", v12, V12);

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

pub(crate) async fn rocket(db_pool: PgPool, http_client: reqwest::Client, rng: Arc<SystemRandom>, port: u16, rooms: Rooms<WebSocket>) -> Result<Rocket<rocket::Ignite>, crate::Error> {
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
    .manage(AtomicUsize::default())
    .ignite().await?)
}
