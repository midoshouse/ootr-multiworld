# runs build commands that may be required by other build commands (since some crates include code from other crates, e.g. updaters)

function ThrowOnNativeFailure {
    if (-not $?)
    {
        throw 'Native Failure'
    }
}

cargo build --package=multiworld-updater
ThrowOnNativeFailure
cargo build --release --package=multiworld-updater
ThrowOnNativeFailure
cargo build --package=multiworld-gui
ThrowOnNativeFailure
cargo build --release --package=multiworld-gui
ThrowOnNativeFailure
cargo build --package=multiworld-csharp
ThrowOnNativeFailure
cargo build --release --package=multiworld-csharp
ThrowOnNativeFailure
cargo build --package=multiworld-bizhawk
ThrowOnNativeFailure
cargo build --release --package=multiworld-bizhawk
ThrowOnNativeFailure
