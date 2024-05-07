#!/bin/sh

set -e

# multiworld-gui
echo 'macOS: building multiworld-gui (debug)'
cargo build --package=multiworld-gui
echo 'macOS: building multiworld-gui (release)'
cargo build --release --package=multiworld-gui

echo 'macOS: bootstrap done'
