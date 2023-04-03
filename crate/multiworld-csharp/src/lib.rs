#![deny(rust_2018_idioms, unused, unused_crate_dependencies, unused_import_braces, unused_lifetimes, unused_qualifications, warnings)]

use {
    std::{
        any::Any,
        convert::TryInto as _,
        ffi::{
            CStr,
            CString,
        },
        fs::{
            self,
            File,
        },
        io::prelude::*,
        num::NonZeroU8,
        slice,
    },
    directories::ProjectDirs,
    iced::{
        Application as _,
        Settings,
        window::{
            self,
            Icon,
        },
    },
    image::ImageFormat,
    libc::c_char,
    once_cell::sync::Lazy,
    ootr_utils::spoiler::HashIcon,
    tokio::sync::mpsc,
    multiworld_derive::csharp_ffi,
    multiworld::config::CONFIG,
    multiworld_gui::{
        FrontendOptions,
        State,
        subscriptions::{
            ClientMessage,
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

fn display_thread_panic(panic_payload: &(dyn Any + Send)) -> String {
    if let Some(msg) = panic_payload.downcast_ref::<&str>() {
        format!("GUI thread panicked: {msg}")
    } else if let Some(msg) = panic_payload.downcast_ref::<String>() {
        format!("GUI thread panicked: {msg}")
    } else {
        format!("GUI thread panicked with a value of type {:?}", panic_payload.type_id())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)] Iced(#[from] iced::Error),
    #[error("{0}")]
    Ffi(String),
    #[error("{}", display_thread_panic(&**.0))]
    Thread(Box<dyn Any + Send>),
}

#[derive(Debug)]
pub struct Client {
    gui_join_handle: std::thread::JoinHandle<iced::Result>,
    rx: mpsc::Receiver<ServerMessage>,
    tx: mpsc::Sender<ClientMessage>,
}

#[derive(Debug)]
pub enum OptMessage {
    None,
    Server(ServerMessage),
    Join,
}

/// # Safety
///
/// `msg` must be a null-terminated UTF-8 string.
#[csharp_ffi] pub unsafe extern "C" fn log(msg: *const c_char) {
    if CONFIG.log {
        writeln!(&*LOG, "{}", CStr::from_ptr(msg).to_str().expect("log text was not valid UTF-8")).expect("failed to write log entry");
    }
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

#[csharp_ffi] pub extern "C" fn open_gui() -> HandleOwned<Client> {
    let (client_tx, client_rx) = mpsc::channel(1_024);
    let (server_tx, server_rx) = mpsc::channel(1_024);
    HandleOwned::new(Client {
        gui_join_handle: std::thread::spawn(move || State::run(Settings {
            window: window::Settings {
                size: (256, 256),
                icon: Icon::from_file_data(include_bytes!("../../../assets/icon.ico"), Some(ImageFormat::Ico)).ok(),
                platform_specific: window::PlatformSpecific {
                    any_thread: true,
                    ..window::PlatformSpecific::default()
                },
                ..window::Settings::default()
            },
            ..Settings::with_flags(FrontendOptions::BizHawk(client_rx, server_tx))
        })),
        rx: server_rx,
        tx: client_tx,
    })
}

/// # Safety
///
/// `s` must point at a valid string. This function takes ownership of the string.
#[csharp_ffi] pub unsafe extern "C" fn string_free(s: StringHandle) {
    let _ = CString::from_raw(s.0);
}

/// # Safety
///
/// `client` must point at a valid `Client`. This function takes ownership of the `Client`.
#[csharp_ffi] pub unsafe extern "C" fn client_free(client: HandleOwned<Client>) {
    let _ = client.into_box();
}

/// # Safety
///
/// `client` must point at a valid `Client`.
///
/// # Panics
///
/// If `id` is `0`.
#[csharp_ffi] pub unsafe extern "C" fn client_set_player_id(client: *mut Client, id: u8) {
    let client = &mut *client;
    let id = NonZeroU8::new(id).expect("tried to claim world 0");
    let _ = client.tx.blocking_send(ClientMessage::PlayerId(id));
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
#[csharp_ffi] pub unsafe extern "C" fn client_reset_player_id(client: *mut Client) {
    let client = &mut *client;
    let _ = client.tx.blocking_send(ClientMessage::ResetPlayerId);
}

/// # Safety
///
/// `client` must point at a valid `Client`. `name` must point at a byte slice of length 8.
#[csharp_ffi] pub unsafe extern "C" fn client_set_player_name(client: *mut Client, name: *const u8) {
    let client = &mut *client;
    let name = slice::from_raw_parts(name, 8);
    let _ = client.tx.blocking_send(ClientMessage::PlayerName(name.try_into().expect("player names are 8 bytes")));
}

/// # Safety
///
/// `client` must point at a valid `Client`. `hash` must point at a byte slice of length 5.
#[csharp_ffi] pub unsafe extern "C" fn client_set_file_hash(client: *mut Client, hash: *const FfiHashIcon) {
    let client = &mut *client;
    let hash = slice::from_raw_parts(hash, 5);
    let _ = client.tx.blocking_send(ClientMessage::FileHash(<[FfiHashIcon; 5]>::try_from(hash).expect("file hashes are 5 bytes").map(HashIcon::from)));
}

/// # Safety
///
/// `client` must point at a valid `Client`. `save` must point at a byte slice of length `0x1450`.
#[csharp_ffi] pub unsafe extern "C" fn client_set_save_data(client: *mut Client, save: *const u8) {
    let client = &mut *client;
    let save = slice::from_raw_parts(save, oottracker::save::SIZE);
    let _ = client.tx.blocking_send(ClientMessage::SaveData(save.try_into().expect("incorrect save data size")));
}

/// Attempts to read a frontend message from the client if one is available, without blocking if there is not.
///
/// # Safety
///
/// `client` must point at a valid `Client`.
#[csharp_ffi] pub unsafe extern "C" fn client_try_recv_message(client: *mut Client) -> HandleOwned<OptMessage> {
    let client = &mut *client;
    HandleOwned::new(match client.rx.try_recv() {
        Ok(msg) => OptMessage::Server(msg),
        Err(_) => if client.gui_join_handle.is_finished() {
            OptMessage::Join
        } else {
            OptMessage::None
        },
    })
}

/// # Safety
///
/// `opt_msg` must point at a valid `OptMessage`. This function takes ownership of the `OptMessage`.
#[csharp_ffi] pub unsafe extern "C" fn opt_message_free(opt_msg: HandleOwned<OptMessage>) {
    let _ = opt_msg.into_box();
}

/// # Safety
///
/// `opt_msg` must point at a valid `OptMessage`.
#[csharp_ffi] pub unsafe extern "C" fn opt_message_kind(opt_msg: *const OptMessage) -> i8 {
    let opt_msg = &*opt_msg;
    match opt_msg {
        OptMessage::None => -1,
        OptMessage::Join => -2,
        OptMessage::Server(ServerMessage::ItemQueue(_)) => 0,
        OptMessage::Server(ServerMessage::GetItem(_)) => 1,
        OptMessage::Server(ServerMessage::PlayerName(_, _)) => 2,
    }
}

/// # Safety
///
/// `opt_msg` must point at a valid `OptMessage`.
///
/// # Panics
///
/// If `opt_msg` is not `Some(ItemQueue(_))`.
#[csharp_ffi] pub unsafe extern "C" fn opt_message_item_queue_len(opt_msg: *const OptMessage) -> u16 {
    let opt_msg = &*opt_msg;
    match opt_msg {
        OptMessage::Server(ServerMessage::ItemQueue(queue)) => queue.len().try_into().expect("too many items in queue"),
        _ => panic!("called opt_message_item_queue_len on {opt_msg:?}"),
    }
}

/// # Safety
///
/// `client` must point at a valid `Client`.
///
/// # Panics
///
/// If `opt_msg` is not `Some(GetItem(_))` or `Some(ItemQueue(_))` or the index is out of range.
#[csharp_ffi] pub unsafe extern "C" fn opt_message_item_kind_at_index(opt_msg: *const OptMessage, index: u16) -> u16 {
    let opt_msg = &*opt_msg;
    match opt_msg {
        OptMessage::Server(ServerMessage::ItemQueue(queue)) => queue[usize::from(index)],
        OptMessage::Server(ServerMessage::GetItem(item)) => *item,
        _ => panic!("called opt_message_item_kind_at_index on {opt_msg:?}"),
    }
}

/// # Safety
///
/// `opt_msg` must point at a valid `OptMessage`.
///
/// # Panics
///
/// If `opt_msg` is not `Some(PlayerName(_, _))`.
#[csharp_ffi] pub unsafe extern "C" fn opt_message_world_id(opt_msg: *const OptMessage) -> u8 {
    let opt_msg = &*opt_msg;
    match opt_msg {
        OptMessage::Server(ServerMessage::PlayerName(world_id, _)) => world_id.get(),
        _ => panic!("called opt_message_world_id on {opt_msg:?}"),
    }
}

/// # Safety
///
/// `opt_msg` must point at a valid `OptMessage`.
///
/// # Panics
///
/// If `opt_msg` is not `Some(PlayerName(_, _))`.
#[csharp_ffi] pub unsafe extern "C" fn opt_message_filename(opt_msg: *const OptMessage) -> *const u8 {
    let opt_msg = &*opt_msg;
    match opt_msg {
        OptMessage::Server(ServerMessage::PlayerName(_, filename)) => filename.0.as_ptr(),
        _ => panic!("called opt_message_world_id on {opt_msg:?}"),
    }
}

/// # Safety
///
/// `client` must point at a valid `Client`.
#[csharp_ffi] pub unsafe extern "C" fn client_send_item(client: *mut Client, key: u32, kind: u16, target_world: u8) {
    let client = &mut *client;
    let target_world = NonZeroU8::new(target_world).expect("tried to send an item to world 0");
    let _ = client.tx.blocking_send(ClientMessage::SendItem { key, kind, target_world });
}

/// # Safety
///
/// `client` must point at a valid `Client`. This function takes ownership of the `Client`.
#[csharp_ffi] pub unsafe extern "C" fn client_join_gui_thread(client: HandleOwned<Client>) -> HandleOwned<Result<(), Error>> {
    let client = client.into_box();
    HandleOwned::new(match client.gui_join_handle.join() {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(e.into()),
        Err(e) => Err(Error::Thread(e)),
    })
}
