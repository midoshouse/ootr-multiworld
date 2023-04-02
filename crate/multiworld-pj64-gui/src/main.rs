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
    let icon = image::load_from_memory(include_bytes!("../../../assets/icon.ico")).expect("failed to load embedded DynamicImage").to_rgba8();
    State::run(Settings {
        window: window::Settings {
            size: (256, 256),
            icon: Some(Icon::from_rgba(icon.as_flat_samples().as_slice().to_owned(), icon.width(), icon.height())?),
            ..window::Settings::default()
        },
        ..Settings::with_flags(FrontendOptions::Pj64(args))
    })?;
    Ok(())
}
