#!/bin/sh

PATH="$HOME/.cargo/bin:$PATH"
set -e

# multiworld-updater
cargo build --package=multiworld-updater
cargo build --release --package=multiworld-updater

# multiworld-gui
cargo build --package=multiworld-gui
cargo build --release --package=multiworld-gui
