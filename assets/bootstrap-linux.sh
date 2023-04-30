#!/bin/sh

PATH="$HOME/.cargo/bin:$PATH"
set -e

# only building with glow backend since the default wgpu backend fails to compile on Linux

# multiworld-updater
cargo build --no-default-features --features=glow --package=multiworld-updater
cargo build --no-default-features --features=glow --release --package=multiworld-updater

# multiworld-gui
cargo build --no-default-features --features=glow --package=multiworld-gui
cargo build --no-default-features --features=glow --release --package=multiworld-gui
