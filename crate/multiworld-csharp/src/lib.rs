use {
    std::{
        convert::TryInto as _,
        env,
        ffi::{
            CStr,
            CString,
        },
        fmt,
        fs::{
            self,
            File,
        },
        io::{
            self,
            prelude::*,
        },
        iter,
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
        str::FromStr as _,
    },
    async_proto::Protocol as _,
    chrono::prelude::*,
    itertools::Itertools as _,
    libc::c_char,
    once_cell::sync::Lazy,
    ootr_utils::spoiler::HashIcon,
    wheel::traits::IoResultExt as _,
    multiworld_derive::csharp_ffi,
    multiworld::{
        HintArea,
        config::Config,
        frontend::{
            ClientMessage,
            PROTOCOL_VERSION,
            ServerMessage,
        },
    },
    crate::util::absolute_path,
};
#[cfg(unix)] use {
    std::os::unix::fs::PermissionsExt as _,
    xdg::BaseDirectories,
};
#[cfg(windows)] use {
    directories::ProjectDirs,
    semver::Version,
};

mod util;

static CONFIG: Lazy<Config> = Lazy::new(|| {
    match Config::blocking_load() {
        Ok(config) => return config,
        #[cfg(debug_assertions)] Err(e) => eprintln!("{e:?}"),
        #[cfg(not(debug_assertions))] Err(_) => {}
    }
    Config::default()
});

static LOG: Lazy<File> = Lazy::new(|| {
    let path = {
        #[cfg(unix)] {
            BaseDirectories::new().expect("failed to determine XDG base directories").place_data_file("midos-house/multiworld-ffi.log").expect("failed to create log dir")
        }
        #[cfg(windows)] {
            let project_dirs = ProjectDirs::from("net", "Fenhl", "OoTR Multiworld").expect("failed to determine project directories");
            fs::create_dir_all(project_dirs.data_dir()).expect("failed to create log dir");
            project_dirs.data_dir().join("ffi.log")
        }
    };
    File::create(path).expect("failed to create log file")
});

#[derive(Debug)]
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

impl<T: fmt::Debug + ?Sized> fmt::Debug for HandleOwned<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        assert!(!self.0.is_null());
        unsafe { (&*self.0).fmt(f) }
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
    #[error(transparent)] ParseInt(#[from] std::num::ParseIntError),
    #[error(transparent)] Read(#[from] async_proto::ReadError),
    #[error(transparent)] Wheel(#[from] wheel::Error),
    #[cfg(windows)] #[error(transparent)] Winver(#[from] winver::Error),
    #[error(transparent)] Write(#[from] async_proto::WriteError),
    #[cfg(unix)] #[error(transparent)] Xdg(#[from] xdg::BaseDirectoriesError),
    #[error("current executable at filesystem root")]
    CurrentExeAtRoot,
    #[error("{0}")]
    Ffi(String),
    #[cfg(target_os = "linux")]
    #[error("could not determine BizHawk location")]
    NoCurrentExe,
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
            tcp_stream.set_nonblocking(true).at_unknown()?;
            ServerMessage::try_read(tcp_stream, &mut self.buf).map_err(Error::from)
        } else {
            match self.tcp_listener.accept() {
                Ok((mut tcp_stream, _)) => {
                    tcp_stream.set_nonblocking(false).at_unknown()?;
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
                Err(e) => Err(e).at_unknown().map_err(Error::from),
            }
        }
    }

    fn write(&mut self, msg: ClientMessage) -> Result<(), Error> {
        if let Some(ref mut tcp_stream) = self.tcp_stream {
            tcp_stream.set_nonblocking(false).at_unknown()?;
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
        writeln!(&*LOG, "{} {}", Utc::now().format("%Y-%m-%d %H:%M:%S"), CStr::from_ptr(msg).to_str().expect("log text was not valid UTF-8")).expect("failed to write log entry");
    }
}

#[csharp_ffi] pub extern "C" fn log_init() {
    if CONFIG.log {
        writeln!(&*LOG, "{} starting Mido's House Multiworld {} for BizHawk", Utc::now().format("%Y-%m-%d %H:%M:%S"), multiworld::version()).expect("failed to write log entry");
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

#[csharp_ffi] pub unsafe extern "C" fn open_gui(version: *const c_char) -> HandleOwned<Result<Client, Error>> {
    fn inner(version: &str) -> Result<Client, Error> {
        let tcp_listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).at_unknown()?;
        tcp_listener.set_nonblocking(true).at_unknown()?;
        let gui_path = {
            #[cfg(unix)] {
                BaseDirectories::new()?.place_cache_file("midos-house/multiworld-gui").at_unknown()?
            }
            #[cfg(windows)] {
                let project_dirs = ProjectDirs::from("net", "Fenhl", "OoTR Multiworld").expect("failed to determine project directories");
                fs::create_dir_all(project_dirs.cache_dir()).at(project_dirs.cache_dir())?;
                project_dirs.cache_dir().join("gui.exe")
            }
        };
        let write_gui = !gui_path.exists() || {
            #[cfg(unix)] { true } //TODO skip if already at the current version (check using --version CLI flag?)
            #[cfg(windows)] {
                let [major, minor, patch, _] = winver::get_file_version_info(&gui_path)?;
                Version::new(major.into(), minor.into(), patch.into()) != multiworld::version()
            }
        };
        if write_gui {
            #[cfg(all(target_arch = "x86_64", target_os = "linux", debug_assertions))] let gui_data = include_bytes!("../../../target/debug/multiworld-gui");
            #[cfg(all(target_arch = "x86_64", target_os = "linux", not(debug_assertions)))] let gui_data = include_bytes!("../../../target/release/multiworld-gui");
            #[cfg(all(target_arch = "x86_64", target_os = "windows", debug_assertions))] let gui_data = include_bytes!("../../../target/debug/multiworld-gui.exe");
            #[cfg(all(target_arch = "x86_64", target_os = "windows", not(debug_assertions)))] let gui_data = include_bytes!("../../../target/release/multiworld-gui.exe");
            fs::write(&gui_path, gui_data).at(&gui_path)?;
            #[cfg(unix)] fs::set_permissions(&gui_path, fs::Permissions::from_mode(0o755)).at(&gui_path)?;
        }
        let (major, minor, patch) = version.split('.').map(u64::from_str).chain(iter::repeat(Ok(0))).next_tuple().expect("iter::repeat produces an infinite iterator");
        let mut cmd = Command::new(&gui_path);
        cmd.arg("bizhawk");
        cmd.arg({
            let emuhawk_path = {
                #[cfg(target_os = "windows")] { env::current_exe().at_unknown()? }
                #[cfg(target_os = "linux")] { env::current_dir().at_unknown()?.join(env::args_os().nth(1).ok_or(Error::NoCurrentExe)?) }
            };
            absolute_path(emuhawk_path)?.parent().ok_or(Error::CurrentExeAtRoot)?
        });
        cmd.arg(process::id().to_string());
        cmd.arg(format!("{}.{}.{}", major?, minor?, patch?));
        cmd.arg(tcp_listener.local_addr().at_unknown()?.port().to_string());
        if CONFIG.log {
            writeln!(&*LOG, "{} running {cmd:?}", Utc::now().format("%Y-%m-%d %H:%M:%S")).expect("failed to write log entry");
        }
        cmd.spawn().at_command(gui_path.display().to_string())?;
        Ok(Client {
            tcp_stream: None,
            buf: Vec::default(),
            message_queue: Vec::default(),
            tcp_listener,
        })
    }

    HandleOwned::new(inner(CStr::from_ptr(version).to_str().expect("version text was not valid UTF-8")))
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
        Err(Error::Read(async_proto::ReadError { kind: async_proto::ReadErrorKind::EndOfStream, .. })) => -3,
        Err(_) => -2,
        Ok(None) => -1,
        Ok(Some(ServerMessage::ItemQueue(_))) => 0,
        Ok(Some(ServerMessage::GetItem(_))) => 1,
        Ok(Some(ServerMessage::PlayerName(_, _))) => 2,
        Ok(Some(ServerMessage::ProgressiveItems(_, _))) => 3,
    }
}

/// # Panics
///
/// If `opt_msg_res` is not `Ok(Some(ItemQueue(_)))`.
#[csharp_ffi] pub unsafe extern "C" fn opt_message_result_item_queue_len(opt_msg_res: *const Result<Option<ServerMessage>, Error>) -> u16 {
    let opt_msg_res = &*opt_msg_res;
    match opt_msg_res {
        Ok(Some(ServerMessage::ItemQueue(queue))) => queue.len().try_into().expect("too many items in queue"),
        _ => panic!("called opt_message_result_item_queue_len on {opt_msg_res:?}"),
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
        _ => panic!("called opt_message_result_item_kind_at_index on {opt_msg_res:?}"),
    }
}

/// # Panics
///
/// If `opt_msg` is not `Some(PlayerName(_, _))`.
#[csharp_ffi] pub unsafe extern "C" fn opt_message_result_world_id(opt_msg_res: *const Result<Option<ServerMessage>, Error>) -> u8 {
    let opt_msg_res = &*opt_msg_res;
    match opt_msg_res {
        Ok(Some(ServerMessage::PlayerName(world_id, _))) | Ok(Some(ServerMessage::ProgressiveItems(world_id, _))) => world_id.get(),
        _ => panic!("called opt_message_result_world_id on {opt_msg_res:?}"),
    }
}

/// # Panics
///
/// If `opt_msg` is not `Some(PlayerName(_, _))`.
#[csharp_ffi] pub unsafe extern "C" fn opt_message_result_filename(opt_msg_res: *const Result<Option<ServerMessage>, Error>) -> *const u8 {
    let opt_msg_res = &*opt_msg_res;
    match opt_msg_res {
        Ok(Some(ServerMessage::PlayerName(_, filename))) => filename.0.as_ptr(),
        _ => panic!("called opt_message_result_filename on {opt_msg_res:?}"),
    }
}

/// # Panics
///
/// If `opt_msg` is not `Some(ProgressiveItems(_, _))`.
#[csharp_ffi] pub unsafe extern "C" fn opt_message_result_progressive_items(opt_msg_res: *const Result<Option<ServerMessage>, Error>) -> u32 {
    let opt_msg_res = &*opt_msg_res;
    match opt_msg_res {
        Ok(Some(ServerMessage::ProgressiveItems(_, progressive_items))) => *progressive_items,
        _ => panic!("called opt_message_result_progressive_items on {opt_msg_res:?}"),
    }
}

#[csharp_ffi] pub unsafe extern "C" fn opt_message_result_unwrap_err(opt_msg_res: HandleOwned<Result<Option<ServerMessage>, Error>>) -> HandleOwned<Error> {
    HandleOwned::new(opt_msg_res.into_box().unwrap_err())
}

#[csharp_ffi] pub unsafe extern "C" fn client_send_item(client: *mut Client, key: u64, kind: u16, target_world: u8) -> HandleOwned<Result<(), Error>> {
    let client = &mut *client;
    let target_world = NonZeroU8::new(target_world).expect("tried to send an item to world 0");
    HandleOwned::new(client.write(ClientMessage::SendItem { key, kind, target_world }))
}

#[allow(dead_code)] // enum variants are constructed by C# code
#[derive(Debug)]
#[repr(u8)]
enum OptHintArea {
    Unknown,
    Root,
    HyruleField,
    LonLonRanch,
    Market,
    TempleOfTime,
    HyruleCastle,
    OutsideGanonsCastle,
    InsideGanonsCastle,
    KokiriForest,
    DekuTree,
    LostWoods,
    SacredForestMeadow,
    ForestTemple,
    DeathMountainTrail,
    DodongosCavern,
    GoronCity,
    DeathMountainCrater,
    FireTemple,
    ZoraRiver,
    ZorasDomain,
    ZorasFountain,
    JabuJabusBelly,
    IceCavern,
    LakeHylia,
    WaterTemple,
    KakarikoVillage,
    BottomOfTheWell,
    Graveyard,
    ShadowTemple,
    GerudoValley,
    GerudoFortress,
    ThievesHideout,
    GerudoTrainingGround,
    HauntedWasteland,
    DesertColossus,
    SpiritTemple,
}

impl TryFrom<OptHintArea> for HintArea {
    type Error = ();

    fn try_from(a: OptHintArea) -> Result<Self, ()> {
        match a {
            OptHintArea::Unknown => Err(()),
            OptHintArea::Root => Ok(Self::Root),
            OptHintArea::HyruleField => Ok(Self::HyruleField),
            OptHintArea::LonLonRanch => Ok(Self::LonLonRanch),
            OptHintArea::Market => Ok(Self::Market),
            OptHintArea::TempleOfTime => Ok(Self::TempleOfTime),
            OptHintArea::HyruleCastle => Ok(Self::HyruleCastle),
            OptHintArea::OutsideGanonsCastle => Ok(Self::OutsideGanonsCastle),
            OptHintArea::InsideGanonsCastle => Ok(Self::InsideGanonsCastle),
            OptHintArea::KokiriForest => Ok(Self::KokiriForest),
            OptHintArea::DekuTree => Ok(Self::DekuTree),
            OptHintArea::LostWoods => Ok(Self::LostWoods),
            OptHintArea::SacredForestMeadow => Ok(Self::SacredForestMeadow),
            OptHintArea::ForestTemple => Ok(Self::ForestTemple),
            OptHintArea::DeathMountainTrail => Ok(Self::DeathMountainTrail),
            OptHintArea::DodongosCavern => Ok(Self::DodongosCavern),
            OptHintArea::GoronCity => Ok(Self::GoronCity),
            OptHintArea::DeathMountainCrater => Ok(Self::DeathMountainCrater),
            OptHintArea::FireTemple => Ok(Self::FireTemple),
            OptHintArea::ZoraRiver => Ok(Self::ZoraRiver),
            OptHintArea::ZorasDomain => Ok(Self::ZorasDomain),
            OptHintArea::ZorasFountain => Ok(Self::ZorasFountain),
            OptHintArea::JabuJabusBelly => Ok(Self::JabuJabusBelly),
            OptHintArea::IceCavern => Ok(Self::IceCavern),
            OptHintArea::LakeHylia => Ok(Self::LakeHylia),
            OptHintArea::WaterTemple => Ok(Self::WaterTemple),
            OptHintArea::KakarikoVillage => Ok(Self::KakarikoVillage),
            OptHintArea::BottomOfTheWell => Ok(Self::BottomOfTheWell),
            OptHintArea::Graveyard => Ok(Self::Graveyard),
            OptHintArea::ShadowTemple => Ok(Self::ShadowTemple),
            OptHintArea::GerudoValley => Ok(Self::GerudoValley),
            OptHintArea::GerudoFortress => Ok(Self::GerudoFortress),
            OptHintArea::ThievesHideout => Ok(Self::ThievesHideout),
            OptHintArea::GerudoTrainingGround => Ok(Self::GerudoTrainingGround),
            OptHintArea::HauntedWasteland => Ok(Self::HauntedWasteland),
            OptHintArea::DesertColossus => Ok(Self::DesertColossus),
            OptHintArea::SpiritTemple => Ok(Self::SpiritTemple),
        }
    }
}

#[csharp_ffi] pub unsafe extern "C" fn client_send_dungeon_reward_info(
    emerald_world: u8, emerald_area: OptHintArea,
    ruby_world: u8, ruby_area: OptHintArea,
    sapphire_world: u8, sapphire_area: OptHintArea,
    light_world: u8, light_area: OptHintArea,
    forest_world: u8, forest_area: OptHintArea,
    fire_world: u8, fire_area: OptHintArea,
    water_world: u8, water_area: OptHintArea,
    shadow_world: u8, shadow_area: OptHintArea,
    spirit_world: u8, spirit_area: OptHintArea,
    client: *mut Client,
) -> HandleOwned<Result<(), Error>> {
    let client = &mut *client;
    HandleOwned::new(client.write(ClientMessage::DungeonRewardInfo {
        emerald: if let (Some(world), Ok(area)) = (NonZeroU8::new(emerald_world), HintArea::try_from(emerald_area)) { Some((world, area)) } else { None },
        ruby: if let (Some(world), Ok(area)) = (NonZeroU8::new(ruby_world), HintArea::try_from(ruby_area)) { Some((world, area)) } else { None },
        sapphire: if let (Some(world), Ok(area)) = (NonZeroU8::new(sapphire_world), HintArea::try_from(sapphire_area)) { Some((world, area)) } else { None },
        light: if let (Some(world), Ok(area)) = (NonZeroU8::new(light_world), HintArea::try_from(light_area)) { Some((world, area)) } else { None },
        forest: if let (Some(world), Ok(area)) = (NonZeroU8::new(forest_world), HintArea::try_from(forest_area)) { Some((world, area)) } else { None },
        fire: if let (Some(world), Ok(area)) = (NonZeroU8::new(fire_world), HintArea::try_from(fire_area)) { Some((world, area)) } else { None },
        water: if let (Some(world), Ok(area)) = (NonZeroU8::new(water_world), HintArea::try_from(water_area)) { Some((world, area)) } else { None },
        shadow: if let (Some(world), Ok(area)) = (NonZeroU8::new(shadow_world), HintArea::try_from(shadow_area)) { Some((world, area)) } else { None },
        spirit: if let (Some(world), Ok(area)) = (NonZeroU8::new(spirit_world), HintArea::try_from(spirit_area)) { Some((world, area)) } else { None },
    }))
}

#[csharp_ffi] pub unsafe extern "C" fn client_send_current_scene(client: *mut Client, current_scene: u8) -> HandleOwned<Result<(), Error>> {
    let client = &mut *client;
    HandleOwned::new(client.write(ClientMessage::CurrentScene(current_scene)))
}
