# runs build commands that may be required by other build commands (since some crates include code from other crates, e.g. updaters)

function ThrowOnNativeFailure {
    if (-not $?)
    {
        throw 'Native Failure'
    }
}

cargo build --release --package=multiworld-updater
ThrowOnNativeFailure
cargo build --release --package=multiworld-pj64-gui
ThrowOnNativeFailure
cargo build --release --package=multiworld-csharp
ThrowOnNativeFailure
cargo build --release --package=multiworld-bizhawk
