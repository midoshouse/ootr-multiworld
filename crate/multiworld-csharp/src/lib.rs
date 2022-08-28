#![deny(rust_2018_idioms, unused, unused_crate_dependencies, unused_import_braces, unused_lifetimes, unused_qualifications, warnings)]

use {
    std::{
        convert::{
            TryFrom as _,
            TryInto as _,
        },
        env,
        ffi::{
            CStr,
            CString,
        },
        fmt,
        fs,
        net::TcpStream,
        num::NonZeroU8,
        process::{
            self,
            Command,
        },
        slice,
        time::Duration,
    },
    async_proto::Protocol,
    directories::ProjectDirs,
    libc::c_char,
    semver::Version,
    multiworld::{
        LobbyClientMessage,
        Player,
        RoomClientMessage,
        ServerMessage,
        format_room_state,
        github::Repo,
    },
};

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

pub struct DebugError(String);

impl<E: fmt::Debug> From<E> for DebugError {
    fn from(e: E) -> DebugError {
        DebugError(format!("{e:?}"))
    }
}

impl fmt::Display for DebugError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// A result type where the error has been converted to its `Debug` representation.
/// Useful because it somewhat deduplicates boilerplate on the C# side.
pub type DebugResult<T> = Result<T, DebugError>;

trait DebugResultExt {
    type T;

    fn debug_unwrap(self) -> Self::T;
}

impl<T> DebugResultExt for DebugResult<T> {
    type T = T;

    fn debug_unwrap(self) -> T {
        match self {
            Ok(x) => x,
            Err(e) => panic!("{e}"),
        }
    }
}

#[derive(Debug)]
pub struct LobbyClient {
    tcp_stream: TcpStream,
    buf: Vec<u8>,
    rooms: Vec<String>,
}

impl LobbyClient {
    fn try_read<T: Protocol>(&mut self) -> Result<Option<T>, async_proto::ReadError> {
        self.tcp_stream.set_nonblocking(true)?;
        T::try_read(&mut self.tcp_stream, &mut self.buf)
    }

    fn write(&mut self, msg: &impl Protocol) -> Result<(), async_proto::WriteError> {
        self.tcp_stream.set_nonblocking(false)?;
        msg.write_sync(&mut self.tcp_stream)
    }
}

#[derive(Debug)]
pub struct RoomClient {
    tcp_stream: TcpStream,
    buf: Vec<u8>,
    players: Vec<Player>,
    num_unassigned_clients: u8,
    last_world: Option<NonZeroU8>,
    last_name: [u8; 8],
    item_queue: Vec<u16>,
}

impl RoomClient {
    fn try_read<T: Protocol>(&mut self) -> Result<Option<T>, async_proto::ReadError> {
        self.tcp_stream.set_nonblocking(true)?;
        T::try_read(&mut self.tcp_stream, &mut self.buf)
    }

    fn write(&mut self, msg: &impl Protocol) -> Result<(), async_proto::WriteError> {
        self.tcp_stream.set_nonblocking(false)?;
        msg.write_sync(&mut self.tcp_stream)
    }
}

#[no_mangle] pub extern "C" fn update_available() -> HandleOwned<DebugResult<bool>> {
    let repo = Repo::new("midoshouse", "ootr-multiworld");
    HandleOwned::new(
        reqwest::blocking::Client::builder()
            .user_agent(concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION")))
            .http2_prior_knowledge()
            .use_rustls_tls()
            .https_only(true)
            .build().map_err(DebugError::from)
            .and_then(|client| repo.latest_release_sync(&client).map_err(DebugError::from))
            .and_then(|release| release.ok_or_else(|| DebugError(format!("no releases"))))
            .and_then(|release| Ok(release.version()? > Version::parse(env!("CARGO_PKG_VERSION")).expect("failed to parse current version")))
    )
}

/// # Safety
///
/// `bool_res` must point at a valid `DebugResult<bool>`. This function takes ownership of the `DebugResult`.
#[no_mangle] pub unsafe extern "C" fn bool_result_free(bool_res: HandleOwned<DebugResult<bool>>) {
    let _ = bool_res.into_box();
}

/// # Safety
///
/// `bool_res` must point at a valid `DebugResult<bool>`.
#[no_mangle] pub unsafe extern "C" fn bool_result_is_ok(bool_res: *const DebugResult<bool>) -> FfiBool {
    (&*bool_res).is_ok().into()
}

/// # Safety
///
/// `bool_res` must point at a valid `DebugResult<bool>`. This function takes ownership of the `DebugResult`.
#[no_mangle] pub unsafe extern "C" fn bool_result_unwrap(bool_res: HandleOwned<DebugResult<bool>>) -> FfiBool {
    bool_res.into_box().debug_unwrap().into()
}

/// # Safety
///
/// `bool_res` must point at a valid `DebugResult<bool>`. This function takes ownership of the `DebugResult`.
#[no_mangle] pub unsafe extern "C" fn bool_result_debug_err(bool_res: HandleOwned<DebugResult<bool>>) -> StringHandle {
    StringHandle::from_string(bool_res.into_box().unwrap_err())
}

#[no_mangle] pub extern "C" fn run_updater() -> HandleOwned<DebugResult<()>> {
    #[cfg(target_os = "windows")] fn inner() -> DebugResult<()> {
        let [major, minor, patch, _] = winver::get_file_version_info("EmuHawk.exe")?;
        let project_dirs = ProjectDirs::from("net", "Fenhl", "OoTR Multiworld").ok_or("user folder not found")?;
        let cache_dir = project_dirs.cache_dir();
        fs::create_dir_all(cache_dir)?;
        let updater_path = cache_dir.join("updater.exe");
        #[cfg(target_arch = "x86_64")] let updater_data = include_bytes!("../../../target/release/multiworld-updater.exe");
        fs::write(&updater_path, updater_data)?;
        Command::new(updater_path)
            .arg(env::current_exe()?.canonicalize()?.parent().ok_or(DebugError(format!("current executable at filesystem root")))?)
            .arg(process::id().to_string())
            .arg(format!("{major}.{minor}.{patch}"))
            .spawn()?;
        Ok(())
    }

    HandleOwned::new(inner())
}

#[no_mangle] pub extern "C" fn connect_ipv4() -> HandleOwned<DebugResult<LobbyClient>> {
    HandleOwned::new(TcpStream::connect((multiworld::ADDRESS_V4, multiworld::PORT))
        .map_err(DebugError::from)
        .and_then(|mut tcp_stream| {
            tcp_stream.set_read_timeout(Some(Duration::from_secs(30)))?;
            tcp_stream.set_write_timeout(Some(Duration::from_secs(30)))?;
            let rooms = multiworld::handshake_sync(&mut tcp_stream)?;
            Ok(LobbyClient {
                buf: Vec::default(),
                rooms: rooms.into_iter().collect(),
                tcp_stream,
            })
        }))
}

#[no_mangle] pub extern "C" fn connect_ipv6() -> HandleOwned<DebugResult<LobbyClient>> {
    HandleOwned::new(TcpStream::connect((multiworld::ADDRESS_V6, multiworld::PORT))
        .map_err(DebugError::from)
        .and_then(|mut tcp_stream| {
            tcp_stream.set_read_timeout(Some(Duration::from_secs(30)))?;
            tcp_stream.set_write_timeout(Some(Duration::from_secs(30)))?;
            let rooms = multiworld::handshake_sync(&mut tcp_stream)?;
            Ok(LobbyClient {
                buf: Vec::default(),
                rooms: rooms.into_iter().collect(),
                tcp_stream,
            })
        }))
}

/// # Safety
///
/// `lobby_client_res` must point at a valid `DebugResult<LobbyClient>`. This function takes ownership of the `DebugResult`.
#[no_mangle] pub unsafe extern "C" fn lobby_client_result_free(lobby_client_res: HandleOwned<DebugResult<LobbyClient>>) {
    let _ = lobby_client_res.into_box();
}

/// # Safety
///
/// `lobby_client_res` must point at a valid `DebugResult<LobbyClient>`.
#[no_mangle] pub unsafe extern "C" fn lobby_client_result_is_ok(lobby_client_res: *const DebugResult<LobbyClient>) -> FfiBool {
    (&*lobby_client_res).is_ok().into()
}

/// # Safety
///
/// `lobby_client_res` must point at a valid `DebugResult<LobbyClient>`. This function takes ownership of the `DebugResult`.
#[no_mangle] pub unsafe extern "C" fn lobby_client_result_unwrap(lobby_client_res: HandleOwned<DebugResult<LobbyClient>>) -> HandleOwned<LobbyClient> {
    HandleOwned::new(lobby_client_res.into_box().debug_unwrap())
}

/// # Safety
///
/// `lobby_client` must point at a valid `LobbyClient`. This function takes ownership of the `LobbyClient`.
#[no_mangle] pub unsafe extern "C" fn lobby_client_free(lobby_client: HandleOwned<LobbyClient>) {
    let _ = lobby_client.into_box();
}

/// # Safety
///
/// `lobby_client_res` must point at a valid `DebugResult<LobbyClient>`. This function takes ownership of the `DebugResult`.
#[no_mangle] pub unsafe extern "C" fn lobby_client_result_debug_err(lobby_client_res: HandleOwned<DebugResult<LobbyClient>>) -> StringHandle {
    StringHandle::from_string(lobby_client_res.into_box().unwrap_err())
}

/// # Safety
///
/// `s` must point at a valid string. This function takes ownership of the string.
#[no_mangle] pub unsafe extern "C" fn string_free(s: StringHandle) {
    let _ = CString::from_raw(s.0);
}

/// # Safety
///
/// `lobby_client` must point at a valid `LobbyClient`.
#[no_mangle] pub unsafe extern "C" fn lobby_client_num_rooms(lobby_client: *const LobbyClient) -> u64 {
    (&*lobby_client).rooms.len().try_into().expect("too many rooms")
}

/// # Safety
///
/// `lobby_client` must point at a valid `LobbyClient`.
///
/// # Panics
///
/// If `i` is out of range.
#[no_mangle] pub unsafe extern "C" fn lobby_client_room_name(lobby_client: *const LobbyClient, i: u64) -> StringHandle {
    StringHandle::from_string(&(&*lobby_client).rooms[usize::try_from(i).expect("index out of range")])
}

/// Attempts to read a message from the server if one is available, without blocking if there is not. Returns the name of the newly opened room, or an empty string if none was opened.
///
/// # Safety
///
/// `lobby_client` must point at a valid `LobbyClient`.
#[no_mangle] pub unsafe extern "C" fn lobby_client_try_recv_new_room(lobby_client: *mut LobbyClient) -> HandleOwned<DebugResult<String>> {
    let lobby_client = &mut *lobby_client;
    HandleOwned::new(match lobby_client.try_read() {
        Ok(Some(ServerMessage::OtherError(e))) => Err(DebugError(e)),
        Ok(Some(ServerMessage::WrongPassword)) => Err(DebugError(format!("wrong password"))),
        Ok(Some(ServerMessage::NewRoom(name))) => {
            if let Err(idx) = lobby_client.rooms.binary_search(&name) {
                lobby_client.rooms.insert(idx, name.clone());
            }
            Ok(name)
        }
        Ok(Some(msg)) => Err(DebugError(format!("{msg:?}"))),
        Ok(None) => Ok(String::default()),
        Err(e) => Err(DebugError::from(e)),
    })
}

/// # Safety
///
/// `str_res` must point at a valid `DebugResult<String>`. This function takes ownership of the `DebugResult`.
#[no_mangle] pub unsafe extern "C" fn string_result_free(str_res: HandleOwned<DebugResult<String>>) {
    let _ = str_res.into_box();
}

/// # Safety
///
/// `str_res` must point at a valid `DebugResult<String>`.
#[no_mangle] pub unsafe extern "C" fn string_result_is_ok(str_res: *const DebugResult<String>) -> FfiBool {
    (&*str_res).is_ok().into()
}

/// # Safety
///
/// `str_res` must point at a valid `DebugResult<String>`. This function takes ownership of the `DebugResult`.
#[no_mangle] pub unsafe extern "C" fn string_result_unwrap(str_res: HandleOwned<DebugResult<String>>) -> StringHandle {
    StringHandle::from_string(str_res.into_box().debug_unwrap())
}

/// # Safety
///
/// `str_res` must point at a valid `DebugResult<String>`. This function takes ownership of the `DebugResult`.
#[no_mangle] pub unsafe extern "C" fn string_result_debug_err(str_res: HandleOwned<DebugResult<String>>) -> StringHandle {
    StringHandle::from_string(str_res.into_box().unwrap_err())
}

/// # Safety
///
/// `lobby_client` must point at a valid `LobbyClient`. This function takes ownership of the `LobbyClient`. `room_name` and `password` must be null-terminated UTF-8 strings.
#[no_mangle] pub unsafe extern "C" fn lobby_client_room_connect(lobby_client: HandleOwned<LobbyClient>, room_name: *const c_char, password: *const c_char) -> HandleOwned<DebugResult<RoomClient>> {
    let mut lobby_client = lobby_client.into_box();
    let name = CStr::from_ptr(room_name).to_str().expect("room name was not valid UTF-8").to_owned();
    let password = CStr::from_ptr(password).to_str().expect("room name was not valid UTF-8");
    HandleOwned::new(if lobby_client.rooms.contains(&name) {
        lobby_client.write(&LobbyClientMessage::JoinRoom { name, password: password.to_owned() })
    } else {
        lobby_client.write(&LobbyClientMessage::CreateRoom { name, password: password.to_owned() })
    }.map_err(DebugError::from)
    .and_then(|()| if lobby_client.buf.is_empty() {
        Ok(())
    } else {
        Err(DebugError(format!("residual data in lobby client buffer upon room join"))) //TODO add blocking read with buffer prefix to async-proto?
    })
    .and_then(|()| loop {
        break match ServerMessage::read_sync(&mut lobby_client.tcp_stream) {
            Ok(ServerMessage::OtherError(e)) => Err(DebugError(e)),
            Ok(ServerMessage::WrongPassword) => Err(DebugError(format!("wrong password"))),
            Ok(ServerMessage::NewRoom(_)) => continue,
            Ok(ServerMessage::EnterRoom { players, num_unassigned_clients }) => Ok((players, num_unassigned_clients)),
            Ok(msg) => Err(DebugError(format!("{msg:?}"))),
            Err(e) => Err(DebugError::from(e)),
        }
    })
    .map(|(players, num_unassigned_clients)| RoomClient {
        players, num_unassigned_clients,
        tcp_stream: lobby_client.tcp_stream,
        buf: Vec::default(),
        last_world: None,
        last_name: Player::DEFAULT_NAME,
        item_queue: Vec::default(),
    }))
}

/// # Safety
///
/// `room_client_res` must point at a valid `DebugResult<RoomClient>`. This function takes ownership of the `DebugResult`.
#[no_mangle] pub unsafe extern "C" fn room_client_result_free(room_client_res: HandleOwned<DebugResult<RoomClient>>) {
    let _ = room_client_res.into_box();
}

/// # Safety
///
/// `room_client_res` must point at a valid `DebugResult<RoomClient>`.
#[no_mangle] pub unsafe extern "C" fn room_client_result_is_ok(room_client_res: *const DebugResult<RoomClient>) -> FfiBool {
    (&*room_client_res).is_ok().into()
}

/// # Safety
///
/// `room_client_res` must point at a valid `DebugResult<RoomClient>`. This function takes ownership of the `DebugResult`.
#[no_mangle] pub unsafe extern "C" fn room_client_result_unwrap(room_client_res: HandleOwned<DebugResult<RoomClient>>) -> HandleOwned<RoomClient> {
    HandleOwned::new(room_client_res.into_box().debug_unwrap())
}

/// # Safety
///
/// `room_client` must point at a valid `RoomClient`. This function takes ownership of the `RoomClient`.
#[no_mangle] pub unsafe extern "C" fn room_client_free(room_client: HandleOwned<RoomClient>) {
    let _ = room_client.into_box();
}

/// # Safety
///
/// `room_client_res` must point at a valid `DebugResult<RoomClient>`. This function takes ownership of the `DebugResult`.
#[no_mangle] pub unsafe extern "C" fn room_client_result_debug_err(room_client_res: HandleOwned<DebugResult<RoomClient>>) -> StringHandle {
    StringHandle::from_string(room_client_res.into_box().unwrap_err())
}

/// # Safety
///
/// `room_client` must point at a valid `RoomClient`.
///
/// # Panics
///
/// If `id` is `0`.
#[no_mangle] pub unsafe extern "C" fn room_client_set_player_id(room_client: *mut RoomClient, id: u8) -> HandleOwned<DebugResult<()>> {
    let room_client = &mut *room_client;
    let id = NonZeroU8::new(id).expect("tried to claim world 0");
    HandleOwned::new(if room_client.last_world != Some(id) {
        room_client.last_world = Some(id);
        room_client.write(&RoomClientMessage::PlayerId(id)).and_then(|()| if room_client.last_name != Player::DEFAULT_NAME {
            room_client.write(&RoomClientMessage::PlayerName(room_client.last_name))
        } else {
            Ok(())
        }).map_err(DebugError::from)
    } else {
        Ok(())
    })
}

/// # Safety
///
/// `unit_res` must point at a valid `DebugResult<()>`. This function takes ownership of the `DebugResult`.
#[no_mangle] pub unsafe extern "C" fn unit_result_free(unit_res: HandleOwned<DebugResult<()>>) {
    let _ = unit_res.into_box();
}

/// # Safety
///
/// `unit_res` must point at a valid `DebugResult<()>`.
#[no_mangle] pub unsafe extern "C" fn unit_result_is_ok(unit_res: *const DebugResult<()>) -> FfiBool {
    (&*unit_res).is_ok().into()
}

/// # Safety
///
/// `unit_res` must point at a valid `DebugResult<()>`. This function takes ownership of the `DebugResult`.
#[no_mangle] pub unsafe extern "C" fn unit_result_debug_err(unit_res: HandleOwned<DebugResult<()>>) -> StringHandle {
    StringHandle::from_string(unit_res.into_box().unwrap_err())
}

/// # Safety
///
/// `room_client` must point at a valid `RoomClient`.
#[no_mangle] pub unsafe extern "C" fn room_client_reset_player_id(room_client: *mut RoomClient) -> HandleOwned<DebugResult<()>> {
    let room_client = &mut *room_client;
    HandleOwned::new(if room_client.last_world != None {
        room_client.last_world = None;
        room_client.write(&RoomClientMessage::ResetPlayerId).map_err(DebugError::from)
    } else {
        Ok(())
    })
}

/// # Safety
///
/// `room_client` must point at a valid `RoomClient`. `name` must point at a byte slice of length 8.
#[no_mangle] pub unsafe extern "C" fn room_client_set_player_name(room_client: *mut RoomClient, name: *const u8) -> HandleOwned<DebugResult<()>> {
    let room_client = &mut *room_client;
    let name = slice::from_raw_parts(name, 8);
    HandleOwned::new(if room_client.last_name != name {
        room_client.last_name = name.try_into().expect("player names are 8 bytes");
        if room_client.last_world.is_some() {
            room_client.write(&RoomClientMessage::PlayerName(room_client.last_name)).map_err(DebugError::from)
        } else {
            Ok(())
        }
    } else {
        Ok(())
    })
}

/// # Safety
///
/// `room_client` must point at a valid `RoomClient`.
#[no_mangle] pub unsafe extern "C" fn room_client_num_players(room_client: *const RoomClient) -> u8 {
    let room_client = &*room_client;
    room_client.players.len().try_into().expect("too many players")
}

/// # Safety
///
/// `room_client` must point at a valid `RoomClient`.
#[no_mangle] pub unsafe extern "C" fn room_client_player_state(room_client: *const RoomClient, player_idx: u8) -> StringHandle {
    let room_client = &*room_client;
    let (mut players, _) = format_room_state(&room_client.players, room_client.num_unassigned_clients, room_client.last_world);
    StringHandle::from_string(players.remove(usize::from(player_idx)))
}

/// # Safety
///
/// `room_client` must point at a valid `RoomClient`.
#[no_mangle] pub unsafe extern "C" fn room_client_other_state(room_client: *const RoomClient) -> StringHandle {
    let room_client = &*room_client;
    let (_, other) = format_room_state(&room_client.players, room_client.num_unassigned_clients, room_client.last_world);
    StringHandle::from_string(other)
}

/// # Safety
///
/// `room_client` must point at a valid `RoomClient`.
#[no_mangle] pub unsafe extern "C" fn room_client_kick_player(room_client: *mut RoomClient, player_idx: u8) -> HandleOwned<DebugResult<()>> {
    let room_client = &mut *room_client;
    let target_world = room_client.players[usize::from(player_idx)].world;
    HandleOwned::new(room_client.write(&RoomClientMessage::KickPlayer(target_world)).map_err(DebugError::from))
}

/// Attempts to read a message from the server if one is available, without blocking if there is not.
///
/// # Safety
///
/// `room_client` must point at a valid `RoomClient`.
#[no_mangle] pub unsafe extern "C" fn room_client_try_recv_message(room_client: *mut RoomClient) -> HandleOwned<DebugResult<Option<ServerMessage>>> {
    let room_client = &mut *room_client;
    HandleOwned::new(match room_client.try_read() {
        Ok(Some(ServerMessage::OtherError(e))) => Err(DebugError(e)),
        Ok(Some(ServerMessage::WrongPassword)) => Err(DebugError(format!("wrong password"))),
        Ok(opt_msg) => Ok(opt_msg),
        Err(e) => Err(DebugError::from(e)),
    })
}

/// # Safety
///
/// `opt_msg_res` must point at a valid `DebugResult<Option<ServerMessage>>`. This function takes ownership of the `DebugResult`.
#[no_mangle] pub unsafe extern "C" fn opt_message_result_free(opt_msg_res: HandleOwned<DebugResult<Option<ServerMessage>>>) {
    let _ = opt_msg_res.into_box();
}

/// # Safety
///
/// `opt_msg_res` must point at a valid `DebugResult<Option<ServerMessage>>`.
#[no_mangle] pub unsafe extern "C" fn opt_message_result_is_ok_some(opt_msg_res: *const DebugResult<Option<ServerMessage>>) -> FfiBool {
    (&*opt_msg_res).as_ref().map_or(false, |opt_msg| opt_msg.is_some()).into()
}

/// # Safety
///
/// `opt_msg_res` must point at a valid `DebugResult<Option<ServerMessage>>>`. This function takes ownership of the `DebugResult`.
#[no_mangle] pub unsafe extern "C" fn opt_message_result_unwrap_unwrap(room_client_res: HandleOwned<DebugResult<Option<ServerMessage>>>) -> HandleOwned<ServerMessage> {
    HandleOwned::new(room_client_res.into_box().debug_unwrap().unwrap())
}

/// # Safety
///
/// `msg` must point at a valid `ServerMessage`. This function takes ownership of the `ServerMessage`.
#[no_mangle] pub unsafe extern "C" fn message_free(msg: HandleOwned<ServerMessage>) {
    let _ = msg.into_box();
}

/// # Safety
///
/// `opt_msg_res` must point at a valid `DebugResult<Option<ServerMessage>>`.
#[no_mangle] pub unsafe extern "C" fn opt_message_result_is_err(opt_msg_res: *const DebugResult<Option<ServerMessage>>) -> FfiBool {
    matches!(&*opt_msg_res, Ok(Some(ServerMessage::OtherError(_) | ServerMessage::WrongPassword)) | Err(_)).into()
}

/// # Safety
///
/// `opt_msg_res` must point at a valid `DebugResult<Option<ServerMessage>>>`. This function takes ownership of the `DebugResult`.
#[no_mangle] pub unsafe extern "C" fn opt_message_result_debug_err(opt_msg_res: HandleOwned<DebugResult<Option<ServerMessage>>>) -> StringHandle {
    StringHandle::from_string(match *opt_msg_res.into_box() {
        Ok(Some(ServerMessage::OtherError(e))) => e,
        Ok(Some(ServerMessage::WrongPassword)) => format!("wrong password"),
        Ok(value) => panic!("tried to debug_err an Ok({value:?})"),
        Err(e) => e.0,
    })
}

/// # Safety
///
/// `msg` must point at a valid `ServerMessage`.
#[no_mangle] pub unsafe extern "C" fn message_effect_type(msg: *const ServerMessage) -> u8 {
    let msg = &*msg;
    match msg {
        ServerMessage::OtherError(_) |
        ServerMessage::NewRoom(_) |
        ServerMessage::AdminLoginSuccess { .. } |
        ServerMessage::WrongPassword => unreachable!(),
        ServerMessage::EnterRoom { .. } |
        ServerMessage::PlayerId(_) |
        ServerMessage::ResetPlayerId(_) |
        ServerMessage::ClientConnected |
        ServerMessage::PlayerDisconnected(_) |
        ServerMessage::UnregisteredClientDisconnected |
        ServerMessage::ItemQueue(_) |
        ServerMessage::GetItem(_) => 0, // changes room state
        ServerMessage::PlayerName(_, _) => 1, // sets a player name and changes room state
    }
}

/// # Safety
///
/// `msg` must point at a valid `ServerMessage`.
///
/// # Panics
///
/// If the `ServerMessage` variant doesn't contain a world ID.
#[no_mangle] pub unsafe extern "C" fn message_player_id(msg: *const ServerMessage) -> u8 {
    let msg = &*msg;
    match msg {
        ServerMessage::PlayerId(world) |
        ServerMessage::ResetPlayerId(world) |
        ServerMessage::PlayerDisconnected(world) |
        ServerMessage::PlayerName(world, _) => world.get(),
        ServerMessage::OtherError(_) |
        ServerMessage::NewRoom(_) |
        ServerMessage::EnterRoom { .. } |
        ServerMessage::ClientConnected |
        ServerMessage::UnregisteredClientDisconnected |
        ServerMessage::ItemQueue(_) |
        ServerMessage::GetItem(_) |
        ServerMessage::AdminLoginSuccess { .. } |
        ServerMessage::WrongPassword => panic!("this message variant has no world ID"),
    }
}

/// # Safety
///
/// `msg` must point at a valid `ServerMessage`.
///
/// # Panics
///
/// If the `ServerMessage` variant doesn't contain a player filename.
#[no_mangle] pub unsafe extern "C" fn message_player_name(msg: *const ServerMessage) -> *const u8 {
    let msg = &*msg;
    if let ServerMessage::PlayerName(_, name) = msg {
        &name[0]
    } else {
        panic!("this message variant has no player filename")
    }
}

/// # Safety
///
/// `room_client` must point at a valid `RoomClient`, and `msg` must point at a valid `ServerMessage`. This function takes ownership of the `ServerMessage`.
#[no_mangle] pub unsafe extern "C" fn room_client_apply_message(room_client: *mut RoomClient, msg: HandleOwned<ServerMessage>) {
    let room_client = &mut *room_client;
    match *msg.into_box() {
        ServerMessage::OtherError(_) | ServerMessage::WrongPassword | ServerMessage::NewRoom(_) | ServerMessage::AdminLoginSuccess { .. } => unreachable!(),
        ServerMessage::EnterRoom { players, num_unassigned_clients } => {
            room_client.players = players;
            room_client.num_unassigned_clients = num_unassigned_clients;
        }
        ServerMessage::PlayerId(world) => if let Err(idx) = room_client.players.binary_search_by_key(&world, |p| p.world) {
            room_client.players.insert(idx, Player::new(world));
            room_client.num_unassigned_clients -= 1;
        },
        ServerMessage::ResetPlayerId(world) => if let Ok(idx) = room_client.players.binary_search_by_key(&world, |p| p.world) {
            room_client.players.remove(idx);
            room_client.num_unassigned_clients += 1;
        },
        ServerMessage::ClientConnected => room_client.num_unassigned_clients += 1,
        ServerMessage::PlayerDisconnected(world) => if let Ok(idx) = room_client.players.binary_search_by_key(&world, |p| p.world) {
            room_client.players.remove(idx);
        },
        ServerMessage::UnregisteredClientDisconnected => room_client.num_unassigned_clients -= 1,
        ServerMessage::PlayerName(world, name) => if let Ok(idx) = room_client.players.binary_search_by_key(&world, |p| p.world) {
            room_client.players[idx].name = name;
        },
        ServerMessage::ItemQueue(queue) => room_client.item_queue = queue,
        ServerMessage::GetItem(item) => room_client.item_queue.push(item),
    }
}

/// # Safety
///
/// `room_client` must point at a valid `RoomClient`.
#[no_mangle] pub unsafe extern "C" fn room_client_send_item(room_client: *mut RoomClient, key: u32, kind: u16, target_world: u8) -> HandleOwned<DebugResult<()>> {
    let room_client = &mut *room_client;
    let target_world = NonZeroU8::new(target_world).expect("tried to send an item to world 0");
    HandleOwned::new(room_client.write(&RoomClientMessage::SendItem { key, kind, target_world }).map_err(DebugError::from))
}

/// # Safety
///
/// `room_client` must point at a valid `RoomClient`.
#[no_mangle] pub unsafe extern "C" fn room_client_item_queue_len(room_client: *const RoomClient) -> u16 {
    let room_client = &*room_client;
    room_client.item_queue.len() as u16
}

/// # Safety
///
/// `room_client` must point at a valid `RoomClient`.
///
/// # Panics
///
/// If `index` is out of range.
#[no_mangle] pub unsafe extern "C" fn room_client_item_kind_at_index(room_client: *const RoomClient, index: u16) -> u16 {
    let room_client = &*room_client;
    room_client.item_queue[usize::from(index)]
}

/// # Safety
///
/// `room_client` must point at a valid `RoomClient`.
///
/// # Panics
///
/// If `world` is `0`.
#[no_mangle] pub unsafe extern "C" fn room_client_get_player_name(room_client: *const RoomClient, world: u8) -> *const u8 {
    let room_client = &*room_client;
    let world = NonZeroU8::new(world).expect("tried to get player name for world 0");
    if let Some(player) = room_client.players.iter().find(|p| p.world == world) {
        &player.name[0]
    } else {
        &Player::DEFAULT_NAME[0]
    }
}
