#!/bin/sh

PATH="$HOME/.cargo/bin:$PATH"
set -e

# multiworld-updater
echo 'WSL: building multiworld-updater (debug)'
cargo build --package=multiworld-updater
echo 'WSL: building multiworld-updater (release)'
cargo build --release --package=multiworld-updater

# multiworld-gui
echo 'WSL: building multiworld-gui (debug)'
cargo build --package=multiworld-gui
echo 'WSL: building multiworld-gui (release)'
cargo build --release --package=multiworld-gui

# multiworld-csharp
echo 'WSL: building multiworld-csharp (debug)'
cargo build --package=multiworld-csharp
echo 'WSL: building multiworld-csharp (release)'
cargo build --release --package=multiworld-csharp

# multiworld-bizhawk
echo 'WSL: building multiworld-bizhawk (debug)'
cargo build --package=multiworld-bizhawk
echo 'WSL: building multiworld-bizhawk (release)'
cargo build --release --package=multiworld-bizhawk

echo 'WSL: bootstrap done'
