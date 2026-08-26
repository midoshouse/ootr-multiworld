#!/bin/sh

set -e

git push
cargo run --release --package=multiworld-release "$@"
