use {
    std::{
        convert::Infallible as Never,
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
    lazy_regex::regex_captures,
    log_lock::*,
    ring::rand::SystemRandom,
    rocket::{
        Rocket,
        State,
        http::Status,
        request::{
            FromRequest,
            Outcome,
            Request,
        },
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
    tokio::sync::watch,
    tokio_tungstenite::tungstenite,
    wheel::traits::IsNetworkError as _,
    multiworld::{
        ClientWriter as _,
        user_agent_hash,
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

#[derive(Debug)]
struct UserAgent(Option<String>);

#[rocket::async_trait]
impl<'r> FromRequest<'r> for UserAgent {
    type Error = Never;

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        Outcome::Success(Self(request.headers().get_one("User-Agent").map(|ua| ua.to_owned())))
    }
}

#[rocket::get("/")]
fn index() -> Redirect {
    Redirect::permanent(uri!("https://midos.house/mw"))
}

macro_rules! supported_version {
    ($endpoint:literal, $version:ident, $variant:ident, $number:literal) => {
        #[rocket::get($endpoint)]
        async fn $version(rng: &State<Arc<SystemRandom>>, db_pool: &State<PgPool>, http_client: &State<reqwest::Client>, rooms: &State<Rooms<WebSocket>>, maintenance: &State<Arc<watch::Sender<Option<(DateTime<Utc>, Duration)>>>>, next_session_id: &State<AtomicUsize>, user_agent: UserAgent, ws: WebSocket, shutdown: rocket::Shutdown) -> rocket_ws::Channel<'static> {
            let _ = sqlx::query!("INSERT INTO mw_versions (version, first_used, last_used) VALUES ($1, NOW(), NOW()) ON CONFLICT (version) DO UPDATE SET last_used = EXCLUDED.last_used", $number).execute(&**db_pool).await;
            let rng = (*rng).clone();
            let db_pool = (*db_pool).clone();
            let http_client = (*http_client).clone();
            let rooms = (*rooms).clone();
            let maintenance = (*maintenance).clone();
            let session_id = next_session_id.fetch_add(1, SeqCst);
            ws.channel(move |stream| Box::pin(async move {
                let version = if let UserAgent(Some(ref user_agent)) = user_agent {
                    if let Some((_, version, found_hash)) = regex_captures!(r"^[0-9A-Za-z_]+/([0-9.]+) \(.+, ([0-9a-f]+)\)$", user_agent) {
                        if let Some(expected_hash) = user_agent_hash(version) {
                            if expected_hash.into_iter().map(|byte| format!("{byte:02x}")).collect::<String>() == found_hash {
                                version.parse().map_err(|_| "failed to parse version")
                            } else {
                                Err("user agent hash mismatch")
                            }
                        } else {
                            Err("server was compiled without user agent salt")
                        }
                    } else {
                        Err("unexpected user agent format")
                    }
                } else {
                    Err("no user agent")
                };
                let (sink, stream) = stream.split();
                let writer = Arc::new(Mutex::new(VersionedWriter { inner: sink, version: Version::$variant }));
                match client_session(&rng, db_pool.clone(), http_client, rooms, session_id, version.clone(), VersionedReader { inner: stream, version: Version::$variant }, Arc::clone(&writer), shutdown, maintenance).await {
                    Ok(()) => {}
                    Err(SessionError::Read(async_proto::ReadError { kind: async_proto::ReadErrorKind::MessageKind021(tungstenite::Message::Close(_)), .. })) => {} // client disconnected normally
                    Err(SessionError::Read(async_proto::ReadError { kind: async_proto::ReadErrorKind::Tungstenite021(tungstenite::Error::Protocol(tungstenite::error::ProtocolError::ResetWithoutClosingHandshake)), .. })) => {} // this happens when a player force quits their multiworld app (or normally quits on macOS, see https://github.com/iced-rs/iced/issues/1941)
                    Err(SessionError::Elapsed(_)) => {} // client not responding
                    Err(SessionError::Shutdown) => {} // server shutting down
                    Err(SessionError::Server(msg)) => {
                        eprintln!("server error in WebSocket handler ({}): {msg}", stringify!($version));
                        if let UserAgent(Some(ref user_agent)) = user_agent {
                            eprintln!("user agent: {user_agent:?}");
                            match version {
                                Ok(version) => {
                                    let _ = wheel::night_report("/games/zelda/oot/mhmw/error", Some(&format!("server error in WebSocket handler (client version {version}): {msg}"))).await;
                                }
                                Err(msg) => eprintln!("{msg}"),
                            }
                        } else {
                            eprintln!("no user agent");
                        }
                        let _ = lock!(writer = writer; writer.write(ServerMessage::OtherError(msg)).await);
                    }
                    Err(e) if e.is_network_error() => {
                        eprintln!("network error in WebSocket handler ({}): {e}", stringify!($version));
                        eprintln!("debug info: {e:?}");
                        let _ = lock!(writer = writer; writer.write(ServerMessage::OtherError(e.to_string())).await);
                    }
                    Err(e) => {
                        eprintln!("error in WebSocket handler ({}): {e}", stringify!($version));
                        if let UserAgent(Some(ref user_agent)) = user_agent {
                            eprintln!("user agent: {user_agent:?}");
                            if let Some((_, version, found_hash)) = regex_captures!(r"^[0-9A-Za-z_]+/([0-9.]+) \(.+, ([0-9a-f]+)\)$", user_agent) {
                                if let Some(expected_hash) = user_agent_hash(version) {
                                    if expected_hash.into_iter().map(|byte| format!("{byte:02x}")).collect::<String>() == found_hash {
                                        let _ = wheel::night_report("/games/zelda/oot/mhmw/error", Some(&format!("error in WebSocket handler (client version {version}): {e}\ndebug info: {e:?}"))).await;
                                    }
                                }
                            }
                        } else {
                            eprintln!("no user agent");
                        }
                        eprintln!("debug info: {e:?}");
                        let _ = lock!(writer = writer; writer.write(ServerMessage::OtherError(e.to_string())).await);
                    }
                }
                let _ = sqlx::query!("INSERT INTO mw_versions (version, first_used, last_used) VALUES ($1, NOW(), NOW()) ON CONFLICT (version) DO UPDATE SET last_used = EXCLUDED.last_used", $number).execute(&db_pool).await;
                Ok(())
            }))
        }
    };
}

macro_rules! unsupported_version {
    ($endpoint:literal, $version:ident) => {
        #[rocket::get($endpoint)]
        async fn $version() -> Status {
            Status::Gone
        }
    };
}

unsupported_version!("/v10", v10);
unsupported_version!("/v11", v11);
unsupported_version!("/v12", v12);
unsupported_version!("/v13", v13);
unsupported_version!("/v14", v14);
supported_version!("/v15", v15, V15, 15);
supported_version!("/v16", v16, V16, 16);
supported_version!("/v17", v17, V17, 17);

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
async fn internal_server_error() -> Result<RawHtml<String>, rocket_util::Error<wheel::Error>> {
    wheel::night_report("/games/zelda/oot/mhmw/error", Some("internal server error")).await?;
    Ok(html! {
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
    })
}

#[rocket::catch(default)]
async fn fallback_catcher(status: Status, _: &Request<'_>) -> Result<RawHtml<String>, rocket_util::Error<wheel::Error>> {
    wheel::night_report("/games/zelda/oot/mhmw/error", Some(&format!("responding with unexpected HTTP status code: {} {}", status.code, status.reason_lossy()))).await?;
    Ok(html! {
        : Doctype;
        html {
            head {
                meta(charset = "utf-8");
                meta(name = "viewport", content = "width=device-width, initial-scale=1, shrink-to-fit=no");
                title {
                    : status.reason_lossy();
                    : " — Mido's House Multiworld";
                };
                link(rel = "icon", sizes = "1024x1024", type = "image/png", href = uri!("https://midos.house/static/mw.png").to_string());
                link(rel = "stylesheet", href = uri!("https://midos.house/static/common.css").to_string());
            }
            body {
                h1 {
                    : "Error ";
                    : status.code;
                    : ": ";
                    : status.reason_lossy();
                }
                p : "Sorry, something went wrong. Please notify Fenhl on Discord.";
            }
        }
    })
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
        fallback_catcher,
    ])
    .manage(db_pool)
    .manage(http_client)
    .manage(rng)
    .manage(rooms)
    .manage(maintenance)
    .manage(AtomicUsize::default())
    .ignite().await?)
}
