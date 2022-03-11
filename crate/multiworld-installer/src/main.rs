#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use {
    directories::UserDirs,
    lazy_regex::regex_is_match,
    native_windows_gui as nwg,
    tokio::fs,
    wheel::traits::IoResultExt as _,
};

const WINDOW_TITLE: &str = "OoTR Multiworld Installer";

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)] FileDialog(#[from] nfd2::error::NfdError),
    #[error(transparent)] Iced(#[from] iced::Error),
    #[error(transparent)] Io(#[from] std::io::Error),
}

#[wheel::main]
async fn main() -> Result<i32, Error> {
    match nwg::message(&nwg::MessageParams {
        title: WINDOW_TITLE,
        content: "Please install BizHawk if you haven't already, then select your BizHawk folder (the one with EmuHawk.exe in it) in the following window.", //TODO ask if BizHawk is installed and offer to install it (and prereqs) if not?
        buttons: nwg::MessageButtons::OkCancel,
        icons: nwg::MessageIcons::Info,
    }) {
        nwg::MessageChoice::Ok => {}
        nwg::MessageChoice::Cancel => return Ok(2),
        _ => unreachable!(),
    }
    // check for existing BizHawk install in Downloads folder (where the bizhawk-co-op install scripts places it)
    let default_bizhawk_dir = UserDirs::new()
        .and_then(|dirs| dirs.download_dir().map(|downloads| downloads.to_owned()))
        .and_then(|downloads| downloads.read_dir().ok())
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name().to_str().map_or(false, |filename| regex_is_match!(r"^BizHawk-[0-9]+(\.[0-9]+){2,3}$", filename)))
        .max_by_key(|entry| entry.file_name())
        .map(|entry| entry.path());
    let bizhawk_dir = match nfd2::open_pick_folder(default_bizhawk_dir.as_deref())? {
        nfd2::Response::Okay(bizhawk_dir) => bizhawk_dir,
        nfd2::Response::OkayMultiple(_) => unreachable!("did not ask for multiple files"),
        nfd2::Response::Cancel => return Ok(2),
    };
    let external_tools_dir = bizhawk_dir.join("ExternalTools");
    fs::create_dir(&external_tools_dir).await.exist_ok()?;
    //TODO download latest release instead of embedding in installer
    fs::write(external_tools_dir.join("multiworld.dll"), include_bytes!("../../../target/release/multiworld.dll")).await?;
    fs::write(external_tools_dir.join("OotrMultiworld.dll"), include_bytes!("../../multiworld-bizhawk/OotrMultiworld/BizHawk/ExternalTools/OotrMultiworld.dll")).await?;
    match nwg::message(&nwg::MessageParams {
        title: WINDOW_TITLE,
        content: "Multiworld has been installed.\n\nTo play multiworld, in BizHawk, select Tools → External Tool → OoTR multiworld.\n\nOpen BizHawk now?",
        buttons: nwg::MessageButtons::YesNo,
        icons: nwg::MessageIcons::Question,
    }) {
        nwg::MessageChoice::Yes => { std::process::Command::new(bizhawk_dir.join("EmuHawk.exe")).arg("--open-ext-tool-dll=OotrMultiworld").current_dir(bizhawk_dir).spawn()?; }
        nwg::MessageChoice::No => {}
        _ => unreachable!(),
    }
    Ok(0)
}
