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
echo 'WSL: bootstrap done'
