#![deny(rust_2018_idioms, unused, unused_crate_dependencies, unused_import_braces, unused_lifetimes, unused_qualifications, warnings)]

use {
    std::{
        collections::HashMap,
        convert::TryInto as _,
        env,
        ffi::{
            CStr,
            CString,
        },
        fs::{
            self,
            File,
        },
        io::prelude::*,
        net::{
            TcpStream,
            ToSocketAddrs,
        },
        num::NonZeroU8,
        process::{
            self,
            Command,
        },
        slice,
        time::{
            Duration,
            Instant,
        },
    },
    async_proto::Protocol,
    directories::ProjectDirs,
    libc::c_char,
    once_cell::sync::Lazy,
    ootr_utils::spoiler::HashIcon,
    semver::Version,
    multiworld_derive::csharp_ffi,
    multiworld::{
        ClientMessage,
        Filename,
        IsNetworkError as _,
        ServerError,
        ServerMessage,
        SessionState,
        SessionStateError,
        config::CONFIG,
        format_room_state,
        github::Repo,
    },
};

static LOG: Lazy<File> = Lazy::new(|| {
    let project_dirs = ProjectDirs::from("net", "Fenhl", "OoTR Multiworld").expect("failed to determine project directories");
    fs::create_dir_all(project_dirs.data_dir()).expect("failed to create log dir");
    File::create(project_dirs.data_dir().join("ffi.log")).expect("failed to create log file")
});

#[repr(transparent)]
pub struct FfiBool(u32);

impl From<bool> for FfiBool {
    fn from(b: bool) -> Self {
        Self(b.into())
    }
}

impl From<FfiBool> for bool {
    fn from(FfiBool(b): FfiBool) -> Self {
        b != 0
    }
}

#[derive(Clone, Copy)]
#[repr(u8)]
pub enum FfiHashIcon {
    DekuStick,
    DekuNut,
    Bow,
    Slingshot,
    FairyOcarina,
    Bombchu,
    Longshot,
    Boomerang,
    LensOfTruth,
    Beans,
    MegatonHammer,
    BottledFish,
    BottledMilk,
    MaskOfTruth,
    SoldOut,
    Cucco,
    Mushroom,
    Saw,
    Frog,
    MasterSword,
    MirrorShield,
    KokiriTunic,
    HoverBoots,
    SilverGauntlets,
    GoldScale,
    StoneOfAgony,
    SkullToken,
    HeartContainer,
    BossKey,
    Compass,
    Map,
    BigMagic,
}

impl From<FfiHashIcon> for HashIcon {
    fn from(icon: FfiHashIcon) -> Self {
        match icon {
            FfiHashIcon::DekuStick => Self::DekuStick,
            FfiHashIcon::DekuNut => Self::DekuNut,
            FfiHashIcon::Bow => Self::Bow,
            FfiHashIcon::Slingshot => Self::Slingshot,
            FfiHashIcon::FairyOcarina => Self::FairyOcarina,
            FfiHashIcon::Bombchu => Self::Bombchu,
            FfiHashIcon::Longshot => Self::Longshot,
            FfiHashIcon::Boomerang => Self::Boomerang,
            FfiHashIcon::LensOfTruth => Self::LensOfTruth,
            FfiHashIcon::Beans => Self::Beans,
            FfiHashIcon::MegatonHammer => Self::MegatonHammer,
            FfiHashIcon::BottledFish => Self::BottledFish,
            FfiHashIcon::BottledMilk => Self::BottledMilk,
            FfiHashIcon::MaskOfTruth => Self::MaskOfTruth,
            FfiHashIcon::SoldOut => Self::SoldOut,
            FfiHashIcon::Cucco => Self::Cucco,
            FfiHashIcon::Mushroom => Self::Mushroom,
            FfiHashIcon::Saw => Self::Saw,
            FfiHashIcon::Frog => Self::Frog,
            FfiHashIcon::MasterSword => Self::MasterSword,
            FfiHashIcon::MirrorShield => Self::MirrorShield,
            FfiHashIcon::KokiriTunic => Self::KokiriTunic,
            FfiHashIcon::HoverBoots => Self::HoverBoots,
            FfiHashIcon::SilverGauntlets => Self::SilverGauntlets,
            FfiHashIcon::GoldScale => Self::GoldScale,
            FfiHashIcon::StoneOfAgony => Self::StoneOfAgony,
            FfiHashIcon::SkullToken => Self::SkullToken,
            FfiHashIcon::HeartContainer => Self::HeartContainer,
            FfiHashIcon::BossKey => Self::BossKey,
            FfiHashIcon::Compass => Self::Compass,
            FfiHashIcon::Map => Self::Map,
            FfiHashIcon::BigMagic => Self::BigMagic,
        }
    }
}

#[repr(transparent)]
pub struct HandleOwned<T: ?Sized>(*mut T);

impl<T> HandleOwned<T> {
    fn new(value: T) -> Self {
        Self(Box::into_raw(Box::new(value)))
    }
}

impl<T: ?Sized> HandleOwned<T> {
    /// # Safety
    ///
    /// `self` must point at a valid `T`. This function takes ownership of the `T`.
    unsafe fn into_box(self) -> Box<T> {
        assert!(!self.0.is_null());
        Box::from_raw(self.0)
    }
}

type StringHandle = HandleOwned<c_char>;

impl StringHandle {
    fn from_string(s: impl ToString) -> Self {
        Self(CString::new(s.to_string()).unwrap().into_raw())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)] Client(#[from] multiworld::ClientError),
    #[error(transparent)] Io(#[from] std::io::Error),
    #[error(transparent)] Json(#[from] serde_json::Error),
    #[error(transparent)] Read(#[from] async_proto::ReadError),
    #[error(transparent)] Reqwest(#[from] reqwest::Error),
    #[error(transparent)] Semver(#[from] semver::Error),
    #[error(transparent)] ServerStructured(#[from] ServerError),
    #[error(transparent)] Winver(#[from] winver::Error),
    #[error(transparent)] Write(#[from] async_proto::WriteError),
    #[error("current executable at filesystem root")]
    CurrentExeAtRoot,
    #[error("{0}")]
    Ffi(String),
    #[error("user folder not found")]
    MissingHomeDir,
    #[error("no releases")]
    NoReleases,
    #[error("tried to connect to a room while not in lobby")]
    NotInLobby,
    #[error("{0}")]
    ServerOther(String),
}

#[derive(Debug)]
pub struct Client {
    session_state: SessionState<String>,
    tcp_stream: TcpStream,
    buf: Vec<u8>,
    retry: Instant,
    wait_time: Duration,
    reconnect: Option<(String, String)>,
    last_ping: Instant,
    last_world: Option<NonZeroU8>,
    last_name: Filename,
    last_hash: Option<[HashIcon; 5]>,
    fallback_filenames: HashMap<NonZeroU8, Filename>,
}

impl Client {
    fn try_read(&mut self) -> Result<Option<ServerMessage>, async_proto::ReadError> {
        self.tcp_stream.set_nonblocking(true)?;
        ServerMessage::try_read(&mut self.tcp_stream, &mut self.buf)
    }

    fn write(&mut self, msg: &ClientMessage) -> Result<(), async_proto::WriteError> {
        self.tcp_stream.set_nonblocking(false)?;
        msg.write_sync(&mut self.tcp_stream)
    }
}

/// # Safety
///
/// `msg` must be a null-terminated UTF-8 string.
#[csharp_ffi] pub unsafe extern "C" fn log(msg: *const c_char) {
    if CONFIG.log {
        writeln!(&*LOG, "{}", CStr::from_ptr(msg).to_str().expect("log text was not valid UTF-8")).expect("failed to write log entry");
    }
}

#[csharp_ffi] pub extern "C" fn version_string() -> StringHandle {
    StringHandle::from_string(if CONFIG.port == multiworld::PORT {
        format!("v{}", env!("CARGO_PKG_VERSION"))
    } else {
        format!("port {} — v{}", CONFIG.port, env!("CARGO_PKG_VERSION"))
    })
}

#[csharp_ffi] pub extern "C" fn update_available() -> HandleOwned<Result<bool, Error>> {
    let repo = Repo::new("midoshouse", "ootr-multiworld");
    HandleOwned::new(
        reqwest::blocking::Client::builder()
            .user_agent(concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION")))
            .http2_prior_knowledge()
            .use_rustls_tls()
            .https_only(true)
            .build().map_err(Error::from)
            .and_then(|client| repo.latest_release_sync(&client).map_err(Error::from))
            .and_then(|release| release.ok_or(Error::NoReleases))
            .and_then(|release| Ok(release.version()? > Version::parse(env!("CARGO_PKG_VERSION")).expect("failed to parse current version")))
    )
}

/// # Safety
///
/// `bool_res` must point at a valid `Result<bool, Error>`. This function takes ownership of the `Result`.
#[csharp_ffi] pub unsafe extern "C" fn bool_result_free(bool_res: HandleOwned<Result<bool, Error>>) {
    let _ = bool_res.into_box();
}

/// # Safety
///
/// `bool_res` must point at a valid `Result<bool, Error>`.
#[csharp_ffi] pub unsafe extern "C" fn bool_result_is_ok(bool_res: *const Result<bool, Error>) -> FfiBool {
    (&*bool_res).is_ok().into()
}

/// # Safety
///
/// `bool_res` must point at a valid `Result<bool, Error>`. This function takes ownership of the `Result`.
#[csharp_ffi] pub unsafe extern "C" fn bool_result_unwrap(bool_res: HandleOwned<Result<bool, Error>>) -> FfiBool {
    bool_res.into_box().unwrap().into()
}

/// # Safety
///
/// `bool_res` must point at a valid `Result<bool>`. This function takes ownership of the `Result`.
#[csharp_ffi] pub unsafe extern "C" fn bool_result_unwrap_err(bool_res: HandleOwned<Result<bool, Error>>) -> HandleOwned<Error> {
    HandleOwned::new(bool_res.into_box().unwrap_err())
}

/// # Safety
///
/// `error` must point at a valid `Error`. This function takes ownership of the `Error`.
#[csharp_ffi] pub unsafe extern "C" fn error_free(error: HandleOwned<Error>) {
    let _ = error.into_box();
}

/// # Safety
///
/// `text` must be a null-terminated UTF-8 string.
#[csharp_ffi] pub unsafe extern "C" fn error_from_string(text: *const c_char) -> HandleOwned<Error> {
    HandleOwned::new(Error::Ffi(CStr::from_ptr(text).to_str().expect("error text was not valid UTF-8").to_owned()))
}

/// # Safety
///
/// `error` must point at a valid `Error`.
#[csharp_ffi] pub unsafe extern "C" fn error_debug(error: *const Error) -> StringHandle {
    let error = &*error;
    StringHandle::from_string(format!("{error:?}"))
}

/// # Safety
///
/// `error` must point at a valid `Error`.
#[csharp_ffi] pub unsafe extern "C" fn error_display(error: *const Error) -> StringHandle {
    let error = &*error;
    StringHandle::from_string(error)
}

#[csharp_ffi] pub extern "C" fn run_updater() -> HandleOwned<Result<(), Error>> {
    #[cfg(target_os = "windows")] fn inner() -> Result<(), Error> {
        let [major, minor, patch, _] = winver::get_file_version_info("EmuHawk.exe")?;
        let project_dirs = ProjectDirs::from("net", "Fenhl", "OoTR Multiworld").ok_or(Error::MissingHomeDir)?;
        let cache_dir = project_dirs.cache_dir();
        fs::create_dir_all(cache_dir)?;
        let updater_path = cache_dir.join("updater.exe");
        #[cfg(all(target_arch = "x86_64", debug_assertions))] let updater_data = include_bytes!("../../../target/debug/multiworld-updater.exe");
        #[cfg(all(target_arch = "x86_64", not(debug_assertions)))] let updater_data = include_bytes!("../../../target/release/multiworld-updater.exe");
        fs::write(&updater_path, updater_data)?;
        Command::new(updater_path)
            .arg("bizhawk")
            .arg(env::current_exe()?.canonicalize()?.parent().ok_or(Error::CurrentExeAtRoot)?)
            .arg(process::id().to_string())
            .arg(format!("{major}.{minor}.{patch}"))
            .spawn()?;
        Ok(())
    }

    HandleOwned::new(inner())
}

#[csharp_ffi] pub extern "C" fn default_port() -> u16 {
    CONFIG.port
}

fn connect_inner(addr: impl ToSocketAddrs) -> Result<TcpStream, Error> {
    TcpStream::connect(addr)
        .map_err(Error::from)
        .and_then(|mut tcp_stream| {
            tcp_stream.set_read_timeout(Some(Duration::from_secs(30)))?;
            tcp_stream.set_write_timeout(Some(Duration::from_secs(30)))?;
            multiworld::handshake_sync(&mut tcp_stream)?;
            Ok(tcp_stream)
        })
}

#[csharp_ffi] pub extern "C" fn connect_ipv4(port: u16) -> HandleOwned<Result<Client, Error>> {
    HandleOwned::new(connect_inner((multiworld::ADDRESS_V4, port)).map(|tcp_stream| Client {
        session_state: SessionState::Init,
        buf: Vec::default(),
        retry: Instant::now(),
        wait_time: Duration::from_secs(1),
        reconnect: None,
        last_ping: Instant::now(),
        last_world: None,
        last_name: Filename::default(),
        last_hash: None,
        fallback_filenames: HashMap::default(),
        tcp_stream,
    }))
}

#[csharp_ffi] pub extern "C" fn connect_ipv6(port: u16) -> HandleOwned<Result<Client, Error>> {
    HandleOwned::new(connect_inner((multiworld::ADDRESS_V6, port)).map(|tcp_stream| Client {
        session_state: SessionState::Init,
        buf: Vec::default(),
        retry: Instant::now(),
        wait_time: Duration::from_secs(1),
        reconnect: None,
        last_ping: Instant::now(),
        last_world: None,
        last_name: Filename::default(),
        last_hash: None,
        fallback_filenames: HashMap::default(),
        tcp_stream,
    }))
}

/// # Safety
///
/// `s` must point at a valid string. This function takes ownership of the string.
#[csharp_ffi] pub unsafe extern "C" fn string_free(s: StringHandle) {
    let _ = CString::from_raw(s.0);
}

/// # Safety
///
/// `client` must point at a valid `Client`.
///
/// # Panics
///
/// If `client` is not in the lobby.
#[csharp_ffi] pub unsafe extern "C" fn client_num_rooms(client: *const Client) -> u64 {
    let client = &*client;
    if let SessionState::Lobby { ref rooms, .. } = client.session_state {
        rooms.len().try_into().expect("too many rooms")
    } else {
        panic!("client is not in the lobby")
    }
}

/// # Safety
///
/// `client` must point at a valid `Client`.
///
/// # Panics
///
/// If `client` is not in the lobby or `i` is out of range.
#[csharp_ffi] pub unsafe extern "C" fn client_room_name(client: *const Client, i: u64) -> StringHandle {
    let client = &*client;
    if let SessionState::Lobby { ref rooms, .. } = client.session_state {
        StringHandle::from_string(rooms.iter().nth(i.try_into().expect("index out of range")).expect("index out of range"))
    } else {
        panic!("client is not in the lobby")
    }
}

/// # Safety
///
/// `str_res` must point at a valid `Result<String, Error>`. This function takes ownership of the `Result`.
#[csharp_ffi] pub unsafe extern "C" fn string_result_free(str_res: HandleOwned<Result<String, Error>>) {
    let _ = str_res.into_box();
}

/// # Safety
///
/// `str_res` must point at a valid `Result<String, Error>`.
#[csharp_ffi] pub unsafe extern "C" fn string_result_is_ok(str_res: *const Result<String, Error>) -> FfiBool {
    (&*str_res).is_ok().into()
}

/// # Safety
///
/// `str_res` must point at a valid `Result<String, Error>`. This function takes ownership of the `Result`.
#[csharp_ffi] pub unsafe extern "C" fn string_result_unwrap(str_res: HandleOwned<Result<String, Error>>) -> StringHandle {
    StringHandle::from_string(str_res.into_box().unwrap())
}

/// # Safety
///
/// `str_res` must point at a valid `Result<String, Error>`. This function takes ownership of the `Result`.
#[csharp_ffi] pub unsafe extern "C" fn string_result_unwrap_err(str_res: HandleOwned<Result<String, Error>>) -> HandleOwned<Error> {
    HandleOwned::new(str_res.into_box().unwrap_err())
}

fn client_room_connect_inner(client: &mut Client, room_name: String, room_password: String) -> Result<(), Error> {
    if let SessionState::Lobby { ref rooms, .. } = client.session_state {
        if rooms.contains(&room_name) {
            client.write(&ClientMessage::JoinRoom { name: room_name.clone(), password: Some(room_password.clone()) })?;
        } else {
            client.write(&ClientMessage::CreateRoom { name: room_name.clone(), password: room_password.clone() })?;
        }
        Ok(())
    } else {
        Err(Error::NotInLobby)
    }
}

/// # Safety
///
/// `client` must point at a valid `Client`. `room_name` and `password` must be null-terminated UTF-8 strings.
#[csharp_ffi] pub unsafe extern "C" fn client_room_connect(client: *mut Client, room_name: *const c_char, room_password: *const c_char) -> HandleOwned<Result<(), Error>> {
    HandleOwned::new(client_room_connect_inner(
        &mut *client,
        CStr::from_ptr(room_name).to_str().expect("room name was not valid UTF-8").to_owned(),
        CStr::from_ptr(room_password).to_str().expect("room name was not valid UTF-8").to_owned(),
    ))
}

/// # Safety
///
/// `client_res` must point at a valid `Result<Client>`. This function takes ownership of the `Result`.
#[csharp_ffi] pub unsafe extern "C" fn client_result_free(client_res: HandleOwned<Result<Client, Error>>) {
    let _ = client_res.into_box();
}

/// # Safety
///
/// `client_res` must point at a valid `Result<Client>`.
#[csharp_ffi] pub unsafe extern "C" fn client_result_is_ok(client_res: *const Result<Client, Error>) -> FfiBool {
    (&*client_res).is_ok().into()
}

/// # Safety
///
/// `client_res` must point at a valid `Result<Client>`. This function takes ownership of the `Result`.
#[csharp_ffi] pub unsafe extern "C" fn client_result_unwrap(client_res: HandleOwned<Result<Client, Error>>) -> HandleOwned<Client> {
    HandleOwned::new(client_res.into_box().unwrap())
}

/// # Safety
///
/// `client` must point at a valid `Client`.
#[csharp_ffi] pub unsafe extern "C" fn client_set_error(client: *mut Client, msg: *const c_char) {
    let client = &mut *client;
    client.session_state = SessionState::Error {
        e: CStr::from_ptr(msg).to_str().expect("error message was not valid UTF-8").to_owned().into(),
        auto_retry: false,
    };
}

/// # Safety
///
/// `client` must point at a valid `Client`.
#[csharp_ffi] pub unsafe extern "C" fn client_session_state(client: *const Client) -> u8 {
    let client = &*client;
    match client.session_state {
        SessionState::Error { .. } => 0,
        SessionState::Init => 1,
        SessionState::InitAutoRejoin { .. } => 2,
        SessionState::Lobby { .. } => 3,
        SessionState::Room { .. } => 4,
        SessionState::Closed => 5,
    }
}

/// # Safety
///
/// `client` must point at a valid `Client`.
#[csharp_ffi] pub unsafe extern "C" fn client_debug_err(client: *const Client) -> StringHandle {
    let client = &*client;
    match client.session_state {
        SessionState::Error { ref e, .. } => StringHandle::from_string(format!("{e:?}")),
        _ => StringHandle::from_string("tried to check session error when there was none"),
    }
}

/// # Safety
///
/// `client` must point at a valid `Client`.
#[csharp_ffi] pub unsafe extern "C" fn client_display_err(client: *const Client) -> StringHandle {
    let client = &*client;
    match client.session_state {
        SessionState::Error { ref e, .. } => StringHandle::from_string(e),
        _ => StringHandle::from_string("tried to check session error when there was none"),
    }
}

/// # Safety
///
/// `client` must point at a valid `Client`.
#[csharp_ffi] pub unsafe extern "C" fn client_has_wrong_password(client: *const Client) -> FfiBool {
    let client = &*client;
    if let SessionState::Lobby { wrong_password, .. } = client.session_state {
        wrong_password
    } else {
        false
    }.into()
}

/// # Safety
///
/// `client` must point at a valid `Client`.
///
/// # Panics
///
/// If `client` is not in the lobby.
#[csharp_ffi] pub unsafe extern "C" fn client_reset_wrong_password(client: *mut Client) {
    let client = &mut *client;
    if let SessionState::Lobby { ref mut wrong_password, .. } = client.session_state {
        *wrong_password = false;
    } else {
        panic!("client is not in the lobby")
    }
}

/// # Safety
///
/// `client` must point at a valid `Client`.
#[csharp_ffi] pub unsafe extern "C" fn client_has_wrong_file_hash(client: *const Client) -> FfiBool {
    let client = &*client;
    if let SessionState::Room { wrong_file_hash, .. } = client.session_state {
        wrong_file_hash
    } else {
        false
    }.into()
}

/// # Safety
///
/// `client` must point at a valid `Client`. This function takes ownership of the `Client`.
#[csharp_ffi] pub unsafe extern "C" fn client_free(client: HandleOwned<Client>) {
    let _ = client.into_box();
}

/// # Safety
///
/// `client_res` must point at a valid `Result<Client, Error>`. This function takes ownership of the `Result`.
#[csharp_ffi] pub unsafe extern "C" fn client_result_unwrap_err(client_res: HandleOwned<Result<Client, Error>>) -> HandleOwned<Error> {
    HandleOwned::new(client_res.into_box().unwrap_err())
}

/// # Safety
///
/// `client` must point at a valid `Client`.
///
/// # Panics
///
/// If `id` is `0`.
#[csharp_ffi] pub unsafe extern "C" fn client_set_player_id(client: *mut Client, id: u8) -> HandleOwned<Result<(), Error>> {
    let client = &mut *client;
    let id = NonZeroU8::new(id).expect("tried to claim world 0");

    if client.last_world != Some(id) {
        let (new_player_name, new_file_hash) = if client.last_world.replace(id).is_none() {
            (
                (client.last_name != Filename::default()).then_some(client.last_name),
                client.last_hash,
            )
        } else {
            (None, None)
        };
        if let SessionState::Room { .. } = client.session_state {
            if let Err(e) = client.write(&ClientMessage::PlayerId(id)) {
                return HandleOwned::new(Err(e.into()))
            }
            if let Some(new_player_name) = new_player_name {
                if let Err(e) = client.write(&ClientMessage::PlayerName(new_player_name)) {
                    return HandleOwned::new(Err(e.into()))
                }
            }
            if let Some(new_file_hash) = new_file_hash {
                if let Err(e) = client.write(&ClientMessage::FileHash(new_file_hash)) {
                    return HandleOwned::new(Err(e.into()))
                }
            }
        }
    }
    HandleOwned::new(Ok(()))
}

/// # Safety
///
/// `unit_res` must point at a valid `Result<(), Error>`. This function takes ownership of the `Result`.
#[csharp_ffi] pub unsafe extern "C" fn unit_result_free(unit_res: HandleOwned<Result<(), Error>>) {
    let _ = unit_res.into_box();
}

/// # Safety
///
/// `unit_res` must point at a valid `Result<(), Error>`.
#[csharp_ffi] pub unsafe extern "C" fn unit_result_is_ok(unit_res: *const Result<(), Error>) -> FfiBool {
    (&*unit_res).is_ok().into()
}

/// # Safety
///
/// `unit_res` must point at a valid `Result<(), Error>`. This function takes ownership of the `Result`.
#[csharp_ffi] pub unsafe extern "C" fn unit_result_unwrap_err(unit_res: HandleOwned<Result<(), Error>>) -> HandleOwned<Error> {
    HandleOwned::new(unit_res.into_box().unwrap_err())
}

/// # Safety
///
/// `client` must point at a valid `Client`.
#[csharp_ffi] pub unsafe extern "C" fn client_reset_player_id(client: *mut Client) -> HandleOwned<Result<(), Error>> {
    let client = &mut *client;
    HandleOwned::new(if client.last_world != None {
        client.last_world = None;
        client.write(&ClientMessage::ResetPlayerId).map_err(Error::from)
    } else {
        Ok(())
    })
}

/// # Safety
///
/// `client` must point at a valid `Client`. `name` must point at a byte slice of length 8.
#[csharp_ffi] pub unsafe extern "C" fn client_set_player_name(client: *mut Client, name: *const u8) -> HandleOwned<Result<(), Error>> {
    let client = &mut *client;
    let name = slice::from_raw_parts(name, 8);

    if client.last_name != name {
        client.last_name = name.try_into().expect("player names are 8 bytes");
        if client.last_world.is_some() {
            if let SessionState::Room { .. } = client.session_state {
                if let Err(e) = client.write(&ClientMessage::PlayerName(client.last_name)) {
                    return HandleOwned::new(Err(e.into()))
                }
            }
        }
    }
    HandleOwned::new(Ok(()))
}

/// # Safety
///
/// `client` must point at a valid `Client`. `hash` must point at a byte slice of length 5.
#[csharp_ffi] pub unsafe extern "C" fn client_set_file_hash(client: *mut Client, hash: *const FfiHashIcon) -> HandleOwned<Result<(), Error>> {
    let client = &mut *client;
    let hash = slice::from_raw_parts(hash, 5);

    let hash = <[FfiHashIcon; 5]>::try_from(hash).expect("file hashes are 5 bytes").map(HashIcon::from);
    if client.last_hash != Some(hash) {
        client.last_hash = Some(hash);
        if client.last_world.is_some() {
            if let SessionState::Room { .. } = client.session_state {
                if let Err(e) = client.write(&ClientMessage::FileHash(hash)) {
                    return HandleOwned::new(Err(e.into()))
                }
            }
        }
    }
    HandleOwned::new(Ok(()))
}

/// # Safety
///
/// `client` must point at a valid `Client`. `save` must point at a byte slice of length `0x1450`.
///
/// # Panics
///
/// If `save` does not represent valid OoT save sata.
#[csharp_ffi] pub unsafe extern "C" fn client_set_save_data(client: *mut Client, save: *const u8) -> HandleOwned<Result<(), Error>> {
    let client = &mut *client;
    let save = slice::from_raw_parts(save, oottracker::save::SIZE);
    if let SessionState::Room { .. } = client.session_state {
        let msg = match oottracker::Save::from_save_data(save) {
            Ok(save) => ClientMessage::SaveData(save),
            Err(e) => ClientMessage::SaveDataError {
                debug: format!("{e:?}"),
                version: multiworld::version(),
            },
        };
        if let Err(e) = client.write(&msg) {
            return HandleOwned::new(Err(e.into()))
        }
    }
    HandleOwned::new(Ok(()))
}

/// # Safety
///
/// `client` must point at a valid `Client`.
#[csharp_ffi] pub unsafe extern "C" fn client_num_players(client: *const Client) -> u8 {
    let client = &*client;
    if let SessionState::Room { ref players, .. } = client.session_state {
        players.len().try_into().expect("too many players")
    } else {
        0
    }
}

/// # Safety
///
/// `client` must point at a valid `Client`.
///
/// # Panics
///
/// If `client` is not in a room or `player_idx` is out of range.
#[csharp_ffi] pub unsafe extern "C" fn client_player_world(client: *const Client, player_idx: u8) -> u8 {
    let client = &*client;
    if let SessionState::Room { ref players, num_unassigned_clients, .. } = client.session_state {
        let (mut players, _) = format_room_state(players, num_unassigned_clients, client.last_world);
        players.remove(usize::from(player_idx)).0.get()
    } else {
        panic!("client is not in a room")
    }
}

/// # Safety
///
/// `client` must point at a valid `Client`.
///
/// # Panics
///
/// If `client` is not in a room or `player_idx` is out of range.
#[csharp_ffi] pub unsafe extern "C" fn client_player_state(client: *const Client, player_idx: u8) -> StringHandle {
    let client = &*client;
    if let SessionState::Room { ref players, num_unassigned_clients, .. } = client.session_state {
        let (mut players, _) = format_room_state(players, num_unassigned_clients, client.last_world);
        StringHandle::from_string(players.remove(usize::from(player_idx)).1)
    } else {
        panic!("client is not in a room")
    }
}

/// # Safety
///
/// `client` must point at a valid `Client`.
///
/// # Panics
///
/// If `client` is not in a room.
#[csharp_ffi] pub unsafe extern "C" fn client_other_room_state(client: *const Client) -> StringHandle {
    let client = &*client;
    if let SessionState::Room { ref players, num_unassigned_clients, .. } = client.session_state {
        let (_, other) = format_room_state(players, num_unassigned_clients, client.last_world);
        StringHandle::from_string(other)
    } else {
        panic!("client is not in a room")
    }
}

/// # Safety
///
/// `client` must point at a valid `Client`.
///
/// # Panics
///
/// If `client` is not in a room.
#[csharp_ffi] pub unsafe extern "C" fn client_kick_player(client: *mut Client, player_idx: u8) -> HandleOwned<Result<(), Error>> {
    let client = &mut *client;
    if let SessionState::Room { ref players, .. } = client.session_state {
        let target_world = players[usize::from(player_idx)].world;
        HandleOwned::new(client.write(&ClientMessage::KickPlayer(target_world)).map_err(Error::from))
    } else {
        panic!("client is not in a room")
    }
}

/// # Safety
///
/// `client` must point at a valid `Client`.
#[csharp_ffi] pub unsafe extern "C" fn client_delete_room(client: *mut Client) -> HandleOwned<Result<(), Error>> {
    let client = &mut *client;
    HandleOwned::new(client.write(&ClientMessage::DeleteRoom).map_err(Error::from))
}

/// Attempts to read a message from the server if one is available, without blocking if there is not.
///
/// # Safety
///
/// `client` must point at a valid `Client`.
#[csharp_ffi] pub unsafe extern "C" fn client_try_recv_message(client: *mut Client, port: u16) -> HandleOwned<Result<Option<ServerMessage>, Error>> {
    let client = &mut *client;
    HandleOwned::new(if let SessionState::Error { auto_retry: true, .. } = client.session_state {
        if client.retry <= Instant::now() {
            match connect_inner((multiworld::ADDRESS_V6, port)).or_else(|_| connect_inner((multiworld::ADDRESS_V4, port))) {
                Ok(tcp_stream) => {
                    client.tcp_stream = tcp_stream;
                    if let Some((room_name, room_password)) = client.reconnect.take() {
                        if let Err(e) = client_room_connect_inner(client, room_name, room_password) {
                            return HandleOwned::new(Err(e))
                        }
                    }
                }
                Err(e) => return HandleOwned::new(Err(e)),
            };
            Ok(None)
        } else {
            Ok(None)
        }
    } else {
        if client.last_ping.elapsed() >= Duration::from_secs(30) {
            if let Err(e) = client.write(&ClientMessage::Ping) {
                return HandleOwned::new(Err(Error::from(e)))
            }
            client.last_ping = Instant::now();
        }
        match client.try_read() {
            Ok(None | Some(ServerMessage::Ping)) => Ok(None),
            Ok(Some(msg)) => {
                client.session_state.apply(msg.clone());
                match msg {
                    ServerMessage::EnterLobby { .. } => if let SessionState::Lobby { existing_room_selection: Some(ref room_name), ref password, .. } = client.session_state {
                        if let Err(e) = client_room_connect_inner(client, room_name.clone(), password.clone()) {
                            return HandleOwned::new(Err(e))
                        }
                    },
                    ServerMessage::EnterRoom { .. } => if let Some(last_world) = client.last_world {
                        if let Err(e) = client.write(&ClientMessage::PlayerId(last_world)) {
                            return HandleOwned::new(Err(Error::from(e)))
                        }
                        if client.last_name != Filename::default() {
                            if let Err(e) = client.write(&ClientMessage::PlayerName(client.last_name)) {
                                return HandleOwned::new(Err(Error::from(e)))
                            }
                        }
                        if let Some(hash) = client.last_hash {
                            if let Err(e) = client.write(&ClientMessage::FileHash(hash)) {
                                return HandleOwned::new(Err(Error::from(e)))
                            }
                        }
                    },
                    ServerMessage::Goodbye => {
                        let _ = client.tcp_stream.shutdown(std::net::Shutdown::Both);
                        client.session_state = SessionState::Closed;
                    }
                    _ => {}
                }
                Ok(Some(msg))
            }
            Err(e) if e.is_network_error() => {
                if client.retry.elapsed() >= Duration::from_secs(60 * 60 * 24) {
                    client.wait_time = Duration::from_secs(1); // reset wait time after no error for a day
                } else {
                    client.wait_time *= 2; // exponential backoff
                }
                client.retry = Instant::now() + client.wait_time;
                if let SessionState::Room { ref room_name, ref room_password, .. } = client.session_state {
                    client.reconnect = Some((room_name.clone(), room_password.clone()));
                }
                client.session_state = SessionState::Error {
                    e: SessionStateError::Connection(e.to_string()),
                    auto_retry: true,
                };
                Ok(None)
            }
            Err(e) => Err(Error::from(e)),
        }
    })
}

/// # Safety
///
/// `opt_msg_res` must point at a valid `Result<Option<ServerMessage>, Error>`. This function takes ownership of the `Result`.
#[csharp_ffi] pub unsafe extern "C" fn opt_message_result_free(opt_msg_res: HandleOwned<Result<Option<ServerMessage>, Error>>) {
    let _ = opt_msg_res.into_box();
}

/// # Safety
///
/// `opt_msg_res` must point at a valid `Result<Option<ServerMessage>, Error>`.
#[csharp_ffi] pub unsafe extern "C" fn opt_message_result_is_ok_some(opt_msg_res: *const Result<Option<ServerMessage>, Error>) -> FfiBool {
    (&*opt_msg_res).as_ref().map_or(false, |opt_msg| opt_msg.is_some()).into()
}

/// # Safety
///
/// `opt_msg_res` must point at a valid `Result<Option<ServerMessage>, Error>`. This function takes ownership of the `Result`.
#[csharp_ffi] pub unsafe extern "C" fn opt_message_result_unwrap_unwrap(opt_msg_res: HandleOwned<Result<Option<ServerMessage>, Error>>) -> HandleOwned<ServerMessage> {
    HandleOwned::new(opt_msg_res.into_box().unwrap().unwrap())
}

/// # Safety
///
/// `msg` must point at a valid `ServerMessage`. This function takes ownership of the `ServerMessage`.
#[csharp_ffi] pub unsafe extern "C" fn message_free(msg: HandleOwned<ServerMessage>) {
    let _ = msg.into_box();
}

/// # Safety
///
/// `opt_msg_res` must point at a valid `Result<Option<ServerMessage>, Error>`.
#[csharp_ffi] pub unsafe extern "C" fn opt_message_result_is_err(opt_msg_res: *const Result<Option<ServerMessage>, Error>) -> FfiBool {
    matches!(&*opt_msg_res, Ok(Some(ServerMessage::StructuredError(_) | ServerMessage::OtherError(_))) | Err(_)).into()
}

/// # Safety
///
/// `opt_msg_res` must point at a valid `Result<Option<ServerMessage>, Error>`. This function takes ownership of the `Result`.
#[csharp_ffi] pub unsafe extern "C" fn opt_message_result_unwrap_err(opt_msg_res: HandleOwned<Result<Option<ServerMessage>, Error>>) -> HandleOwned<Error> {
    HandleOwned::new(match *opt_msg_res.into_box() {
        Ok(Some(ServerMessage::StructuredError(e))) => Error::ServerStructured(e),
        Ok(Some(ServerMessage::OtherError(e))) => Error::ServerOther(e),
        Ok(value) => panic!("tried to unwrap_err an Ok({value:?})"),
        Err(e) => e,
    })
}

/// # Safety
///
/// `client` must point at a valid `Client`.
#[csharp_ffi] pub unsafe extern "C" fn client_send_item(client: *mut Client, key: u32, kind: u16, target_world: u8) -> HandleOwned<Result<(), Error>> {
    let client = &mut *client;
    let target_world = NonZeroU8::new(target_world).expect("tried to send an item to world 0");
    HandleOwned::new(client.write(&ClientMessage::SendItem { key, kind, target_world }).map_err(Error::from))
}

/// # Safety
///
/// `client` must point at a valid `Client`.
///
/// # Panics
///
/// If `client` is not in a room.
#[csharp_ffi] pub unsafe extern "C" fn client_item_queue_len(client: *const Client) -> u16 {
    let client = &*client;
    if let SessionState::Room { ref item_queue, .. } = client.session_state {
        item_queue.len() as u16
    } else {
        panic!("client is not in a room")
    }
}

/// # Safety
///
/// `client` must point at a valid `Client`.
///
/// # Panics
///
/// If `client` is not in a room or `index` is out of range.
#[csharp_ffi] pub unsafe extern "C" fn client_item_kind_at_index(client: *const Client, index: u16) -> u16 {
    let client = &*client;
    if let SessionState::Room { ref item_queue, .. } = client.session_state {
        item_queue[usize::from(index)]
    } else {
        panic!("client is not in a room")
    }
}

/// # Safety
///
/// `client` must point at a valid `Client`.
///
/// # Panics
///
/// If `client` is not in a room or `world` is `0`.
#[csharp_ffi] pub unsafe extern "C" fn client_get_player_name(client: *mut Client, world: u8) -> *const u8 {
    let client = &mut *client;
    let world = NonZeroU8::new(world).expect("tried to get player name for world 0");
    if let SessionState::Room { ref players, .. } = client.session_state {
        if let Some(player) = players.iter().find(|p| p.world == world) {
            player.name.0.as_ptr()
        } else {
            client.fallback_filenames.entry(world).or_insert_with(|| Filename::fallback(world)).0.as_ptr()
        }
    } else {
        panic!("client is not in a room")
    }
}

/// # Safety
///
/// `client` must point at a valid `Client`.
///
/// # Panics
///
/// If `client` is not in a room.
#[csharp_ffi] pub unsafe extern "C" fn client_get_autodelete_seconds(client: *const Client) -> u64 {
    let client = &*client;
    if let SessionState::Room { autodelete_delta, .. } = client.session_state {
        autodelete_delta.as_secs()
    } else {
        panic!("client is not in a room")
    }
}

/// # Safety
///
/// `client` must point at a valid `Client`.
#[csharp_ffi] pub unsafe extern "C" fn client_set_autodelete_seconds(client: *mut Client, seconds: u64) -> HandleOwned<Result<(), Error>> {
    let client = &mut *client;
    HandleOwned::new(client.write(&ClientMessage::AutoDeleteDelta(Duration::from_secs(seconds))).map_err(Error::Write))
}

/// # Safety
///
/// `client` must point at a valid `Client`.
#[csharp_ffi] pub unsafe extern "C" fn client_send_all(client: *mut Client, source_world: u8, spoiler_log_path: *const c_char) -> HandleOwned<Result<(), Error>> {
    let client = &mut *client;
    let source_world = NonZeroU8::new(source_world).expect("tried to send all items from world 0");
    let spoiler_log_path = CStr::from_ptr(spoiler_log_path).to_str().expect("spoiler log path was not valid UTF-8");
    HandleOwned::new(
        fs::read_to_string(spoiler_log_path).map_err(Error::Io)
        .and_then(|spoiler_log| serde_json::from_str(&spoiler_log).map_err(Error::Json))
        .and_then(|spoiler_log| client.write(&ClientMessage::SendAll { source_world, spoiler_log }).map_err(Error::Write))
    )
}
