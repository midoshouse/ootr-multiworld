#![deny(rust_2018_idioms, unused, unused_crate_dependencies, unused_import_braces, unused_lifetimes, warnings)]
#![forbid(unsafe_code)]

use {
    std::io,
    winres::WindowsResource,
};

fn main() -> io::Result<()> {
    WindowsResource::new()
        .set_icon("../../assets/icon.ico")
        .compile()?;
    Ok(())
}
