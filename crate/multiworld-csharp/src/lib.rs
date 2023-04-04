#![deny(rust_2018_idioms, unused, unused_crate_dependencies, unused_import_braces, unused_lifetimes, unused_qualifications, warnings)]

use {
    std::{
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
        io::{
            self,
            prelude::*,
        },
        net::{
            Ipv4Addr,
            TcpListener,
            TcpStream,
        },
        num::NonZeroU8,
        process::{
            self,
            Command,
        },
        slice,
    },
    async_proto::Protocol as _,
    directories::ProjectDirs,
    libc::c_char,
    once_cell::sync::Lazy,
    ootr_utils::spoiler::HashIcon,
    semver::Version,
    multiworld_derive::csharp_ffi,
    multiworld::{
        config::CONFIG,
        frontend::{
            ClientMessage,
            PORT,
            PROTOCOL_VERSION,
            ServerMessage,
        },
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
    #[error(transparent)] Io(#[from] io::Error),
    #[error(transparent)] Read(#[from] async_proto::ReadError),
    #[error(transparent)] Winver(#[from] winver::Error),
    #[error(transparent)] Write(#[from] async_proto::WriteError),
    #[error("current executable at filesystem root")]
    CurrentExeAtRoot,
    #[error("{0}")]
    Ffi(String),
    #[error("protocol version mismatch: multiworld app is version {0} but we're version {}", PROTOCOL_VERSION)]
    VersionMismatch(u8),
}

#[derive(Debug)]
pub struct Client {
    tcp_listener: TcpListener,
    tcp_stream: Option<TcpStream>,
    buf: Vec<u8>,
    message_queue: Vec<ClientMessage>,
}

impl Client {
    fn try_read(&mut self) -> Result<Option<ServerMessage>, Error> {
        if let Some(ref mut tcp_stream) = self.tcp_stream {
            tcp_stream.set_nonblocking(true)?;
            ServerMessage::try_read(tcp_stream, &mut self.buf).map_err(Error::from)
        } else {
            match self.tcp_listener.accept() {
                Ok((mut tcp_stream, _)) => {
                    tcp_stream.set_nonblocking(false)?;
                    PROTOCOL_VERSION.write_sync(&mut tcp_stream)?;
                    let frontend_version = u8::read_sync(&mut tcp_stream)?;
                    if frontend_version != PROTOCOL_VERSION { return Err(Error::VersionMismatch(frontend_version)) }
                    for msg in self.message_queue.drain(..) {
                        msg.write_sync(&mut tcp_stream)?;
                    }
                    self.tcp_stream = Some(tcp_stream);
                    Ok(None)
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(None),
                Err(e) => Err(e.into()),
            }
        }
    }

    fn write(&mut self, msg: ClientMessage) -> Result<(), Error> {
        if let Some(ref mut tcp_stream) = self.tcp_stream {
            tcp_stream.set_nonblocking(false)?;
            msg.write_sync(tcp_stream).map_err(Error::from)
        } else {
            self.message_queue.push(msg);
            Ok(())
        }
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

#[csharp_ffi] pub unsafe extern "C" fn error_free(error: HandleOwned<Error>) {
    let _ = error.into_box();
}

/// # Safety
///
/// `text` must be a null-terminated UTF-8 string.
#[csharp_ffi] pub unsafe extern "C" fn error_from_string(text: *const c_char) -> HandleOwned<Error> {
    HandleOwned::new(Error::Ffi(CStr::from_ptr(text).to_str().expect("error text was not valid UTF-8").to_owned()))
}

#[csharp_ffi] pub unsafe extern "C" fn error_debug(error: *const Error) -> StringHandle {
    let error = &*error;
    StringHandle::from_string(format!("{error:?}"))
}

#[csharp_ffi] pub unsafe extern "C" fn error_display(error: *const Error) -> StringHandle {
    let error = &*error;
    StringHandle::from_string(error)
}

#[csharp_ffi] pub extern "C" fn open_gui() -> HandleOwned<Result<Client, Error>> {
    fn inner() -> Result<Client, Error> {
        let tcp_listener = TcpListener::bind((Ipv4Addr::LOCALHOST, PORT))?;
        tcp_listener.set_nonblocking(true)?;
        let project_dirs = ProjectDirs::from("net", "Fenhl", "OoTR Multiworld").expect("failed to determine project directories");
        let gui_path = project_dirs.cache_dir().join("gui.exe");
        let write_gui = !gui_path.exists() || {
            let [major, minor, patch, _] = winver::get_file_version_info(&gui_path)?;
            Version::new(major.into(), minor.into(), patch.into()) != multiworld::version()
        };
        if write_gui {
            fs::create_dir_all(project_dirs.cache_dir())?;
            #[cfg(all(target_arch = "x86_64", debug_assertions))] let gui_data = include_bytes!("../../../target/debug/multiworld-gui.exe");
            #[cfg(all(target_arch = "x86_64", not(debug_assertions)))] let gui_data = include_bytes!("../../../target/release/multiworld-gui.exe");
            fs::write(&gui_path, gui_data)?;
        }
        let [major, minor, patch, _] = winver::get_file_version_info("EmuHawk.exe")?;
        Command::new(gui_path)
            //TODO forward log and port args from config
            .arg("bizhawk")
            .arg(env::current_exe()?.canonicalize()?.parent().ok_or(Error::CurrentExeAtRoot)?)
            .arg(process::id().to_string())
            .arg(format!("{major}.{minor}.{patch}"))
            .spawn()?;
        Ok(Client {
            tcp_stream: None,
            buf: Vec::default(),
            message_queue: Vec::default(),
            tcp_listener,
        })
    }

    HandleOwned::new(inner())
}

#[csharp_ffi] pub unsafe extern "C" fn client_result_free(client_res: HandleOwned<Result<Client, Error>>) {
    let _ = client_res.into_box();
}

#[csharp_ffi] pub unsafe extern "C" fn client_result_is_ok(client_res: *const Result<Client, Error>) -> FfiBool {
    (&*client_res).is_ok().into()
}

#[csharp_ffi] pub unsafe extern "C" fn client_result_unwrap(client_res: HandleOwned<Result<Client, Error>>) -> HandleOwned<Client> {
    HandleOwned::new(client_res.into_box().unwrap())
}

#[csharp_ffi] pub unsafe extern "C" fn client_result_unwrap_err(client_res: HandleOwned<Result<Client, Error>>) -> HandleOwned<Error> {
    HandleOwned::new(client_res.into_box().unwrap_err())
}

/// # Safety
///
/// `s` must point at a valid UTF-8 string. This function takes ownership of the string.
#[csharp_ffi] pub unsafe extern "C" fn string_free(s: StringHandle) {
    let _ = CString::from_raw(s.0);
}

#[csharp_ffi] pub unsafe extern "C" fn client_free(client: HandleOwned<Client>) {
    let _ = client.into_box();
}

/// # Panics
///
/// If `id` is `0`.
#[csharp_ffi] pub unsafe extern "C" fn client_set_player_id(client: *mut Client, id: u8) -> HandleOwned<Result<(), Error>> {
    let client = &mut *client;
    let id = NonZeroU8::new(id).expect("tried to claim world 0");
    HandleOwned::new(client.write(ClientMessage::PlayerId(id)))
}

#[csharp_ffi] pub unsafe extern "C" fn unit_result_free(unit_res: HandleOwned<Result<(), Error>>) {
    let _ = unit_res.into_box();
}

#[csharp_ffi] pub unsafe extern "C" fn unit_result_is_ok(unit_res: *const Result<(), Error>) -> FfiBool {
    (&*unit_res).is_ok().into()
}

#[csharp_ffi] pub unsafe extern "C" fn unit_result_unwrap_err(unit_res: HandleOwned<Result<(), Error>>) -> HandleOwned<Error> {
    HandleOwned::new(unit_res.into_box().unwrap_err())
}

#[csharp_ffi] pub unsafe extern "C" fn client_reset_player_id(client: *mut Client) -> HandleOwned<Result<(), Error>> {
    let client = &mut *client;
    HandleOwned::new(client.write(ClientMessage::ResetPlayerId))
}

/// # Safety
///
/// `name` must point at a byte slice of length 8.
#[csharp_ffi] pub unsafe extern "C" fn client_set_player_name(client: *mut Client, name: *const u8) -> HandleOwned<Result<(), Error>> {
    let client = &mut *client;
    let name = slice::from_raw_parts(name, 8);
    HandleOwned::new(client.write(ClientMessage::PlayerName(name.try_into().expect("player names are 8 bytes"))))
}

/// # Safety
///
/// `hash` must point at a byte slice of length 5.
#[csharp_ffi] pub unsafe extern "C" fn client_set_file_hash(client: *mut Client, hash: *const FfiHashIcon) -> HandleOwned<Result<(), Error>> {
    let client = &mut *client;
    let hash = slice::from_raw_parts(hash, 5);
    HandleOwned::new(client.write(ClientMessage::FileHash(<[FfiHashIcon; 5]>::try_from(hash).expect("file hashes are 5 bytes").map(HashIcon::from))))
}

/// # Safety
///
/// `save` must point at a byte slice of length `0x1450`.
#[csharp_ffi] pub unsafe extern "C" fn client_set_save_data(client: *mut Client, save: *const u8) -> HandleOwned<Result<(), Error>> {
    let client = &mut *client;
    let save = slice::from_raw_parts(save, oottracker::save::SIZE);
    HandleOwned::new(client.write(ClientMessage::SaveData(save.try_into().expect("incorrect save data size"))))
}

/// Attempts to read a frontend message from the client if one is available, without blocking if there is not.
#[csharp_ffi] pub unsafe extern "C" fn client_try_recv_message(client: *mut Client) -> HandleOwned<Result<Option<ServerMessage>, Error>> {
    let client = &mut *client;
    HandleOwned::new(client.try_read())
}

#[csharp_ffi] pub unsafe extern "C" fn opt_message_result_free(opt_msg_res: HandleOwned<Result<Option<ServerMessage>, Error>>) {
    let _ = opt_msg_res.into_box();
}

#[csharp_ffi] pub unsafe extern "C" fn opt_message_result_kind(opt_msg_res: *const Result<Option<ServerMessage>, Error>) -> i8 {
    let opt_msg_res = &*opt_msg_res;
    match opt_msg_res {
        Err(Error::Read(async_proto::ReadError::EndOfStream)) => -3,
        Err(_) => -2,
        Ok(None) => -1,
        Ok(Some(ServerMessage::ItemQueue(_))) => 0,
        Ok(Some(ServerMessage::GetItem(_))) => 1,
        Ok(Some(ServerMessage::PlayerName(_, _))) => 2,
    }
}

/// # Panics
///
/// If `opt_msg_res` is not `Ok(Some(ItemQueue(_)))`.
#[csharp_ffi] pub unsafe extern "C" fn opt_message_result_item_queue_len(opt_msg_res: *const Result<Option<ServerMessage>, Error>) -> u16 {
    let opt_msg_res = &*opt_msg_res;
    match opt_msg_res {
        Ok(Some(ServerMessage::ItemQueue(queue))) => queue.len().try_into().expect("too many items in queue"),
        _ => panic!("called opt_message_item_queue_len on {opt_msg_res:?}"),
    }
}

/// # Panics
///
/// If `opt_msg_res` is not `Ok(Some(GetItem(_)))` or `Ok(Some(ItemQueue(_)))` or the index is out of range.
#[csharp_ffi] pub unsafe extern "C" fn opt_message_result_item_kind_at_index(opt_msg_res: *const Result<Option<ServerMessage>, Error>, index: u16) -> u16 {
    let opt_msg_res = &*opt_msg_res;
    match opt_msg_res {
        Ok(Some(ServerMessage::ItemQueue(queue))) => queue[usize::from(index)],
        Ok(Some(ServerMessage::GetItem(item))) => *item,
        _ => panic!("called opt_message_item_kind_at_index on {opt_msg_res:?}"),
    }
}

/// # Panics
///
/// If `opt_msg` is not `Some(PlayerName(_, _))`.
#[csharp_ffi] pub unsafe extern "C" fn opt_message_result_world_id(opt_msg_res: *const Result<Option<ServerMessage>, Error>) -> u8 {
    let opt_msg_res = &*opt_msg_res;
    match opt_msg_res {
        Ok(Some(ServerMessage::PlayerName(world_id, _))) => world_id.get(),
        _ => panic!("called opt_message_world_id on {opt_msg_res:?}"),
    }
}

/// # Panics
///
/// If `opt_msg` is not `Some(PlayerName(_, _))`.
#[csharp_ffi] pub unsafe extern "C" fn opt_message_result_filename(opt_msg_res: *const Result<Option<ServerMessage>, Error>) -> *const u8 {
    let opt_msg_res = &*opt_msg_res;
    match opt_msg_res {
        Ok(Some(ServerMessage::PlayerName(_, filename))) => filename.0.as_ptr(),
        _ => panic!("called opt_message_world_id on {opt_msg_res:?}"),
    }
}

#[csharp_ffi] pub unsafe extern "C" fn opt_message_result_unwrap_err(opt_msg_res: HandleOwned<Result<Option<ServerMessage>, Error>>) -> HandleOwned<Error> {
    HandleOwned::new(opt_msg_res.into_box().unwrap_err())
}

#[csharp_ffi] pub unsafe extern "C" fn client_send_item(client: *mut Client, key: u32, kind: u16, target_world: u8) -> HandleOwned<Result<(), Error>> {
    let client = &mut *client;
    let target_world = NonZeroU8::new(target_world).expect("tried to send an item to world 0");
    HandleOwned::new(client.write(ClientMessage::SendItem { key, kind, target_world }))
}
