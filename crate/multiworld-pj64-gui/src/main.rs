#![deny(rust_2018_idioms, unused, unused_crate_dependencies, unused_import_braces, unused_lifetimes, warnings)]
#![forbid(unsafe_code)]

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use {
    iced::{
        Application as _,
        Settings,
        window::{
            self,
            Icon,
        },
    },
    image::ImageFormat,
    multiworld_gui::{
        CliArgs,
        FrontendOptions,
        State,
    },
};

#[derive(Debug, thiserror::Error)]
enum MainError {
    #[error(transparent)] Iced(#[from] iced::Error),
    #[error(transparent)] Icon(#[from] iced::window::icon::Error),
}

#[wheel::main]
fn main(args: CliArgs) -> Result<(), MainError> {
    State::run(Settings {
        window: window::Settings {
            size: (256, 256),
            icon: Some(Icon::from_file_data(include_bytes!("../../../assets/icon.ico"), Some(ImageFormat::Ico))?),
            ..window::Settings::default()
        },
        ..Settings::with_flags(FrontendOptions::Pj64(args))
    })?;
    Ok(())
}
