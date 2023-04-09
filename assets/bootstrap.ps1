# runs build commands that may be required by other build commands (since some crates include code from other crates, e.g. updaters)

function ThrowOnNativeFailure {
    if (-not $?)
    {
        throw 'Native Failure'
    }
}

# multiworld-updater
cargo build --no-default-features --features=glow --target-dir=target/glow --package=multiworld-updater
ThrowOnNativeFailure
cargo build --no-default-features --features=glow --target-dir=target/glow --release --package=multiworld-updater
ThrowOnNativeFailure
cargo build --package=multiworld-updater
ThrowOnNativeFailure
cargo build --release --package=multiworld-updater
ThrowOnNativeFailure

# multiworld-gui
cargo build --no-default-features --features=glow --target-dir=target/glow --package=multiworld-gui
ThrowOnNativeFailure
cargo build --no-default-features --features=glow --target-dir=target/glow --release --package=multiworld-gui
ThrowOnNativeFailure
cargo build --package=multiworld-gui
ThrowOnNativeFailure
cargo build --release --package=multiworld-gui
ThrowOnNativeFailure

# multiworld-csharp
cargo build --package=multiworld-csharp
ThrowOnNativeFailure
cargo build --release --package=multiworld-csharp
ThrowOnNativeFailure

# multiworld-bizhawk
cargo build --package=multiworld-bizhawk
ThrowOnNativeFailure
cargo build --release --package=multiworld-bizhawk
ThrowOnNativeFailure

# multiworld-installer
cargo build --no-default-features --features=glow --target-dir=target/glow --package=multiworld-installer
ThrowOnNativeFailure
cargo build --no-default-features --features=glow --target-dir=target/glow --release --package=multiworld-installer
ThrowOnNativeFailure
