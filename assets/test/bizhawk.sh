set -e

#TODO make sure BizHawk is up to date

cargo build --package=multiworld-gui
cargo build --package=multiworld-csharp
cargo build --package=multiworld-bizhawk
env -C crate/multiworld-bizhawk/OotrMultiworld/BizHawk ./EmuHawkMono.sh --mono-no-redirect --open-ext-tool-dll=OotrMultiworld
