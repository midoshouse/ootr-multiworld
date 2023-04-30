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

# Linux
# build on Debian because Ubuntu's glibc is too new for compatibility with Debian
debian run sudo apt-get install -y rsync
debian run mkdir -p /home/fenhl/wslgit/github.com/midoshouse
debian run rsync --delete -av /mnt/c/Users/fenhl/git/github.com/midoshouse/ootr-multiworld/stage/ /home/fenhl/wslgit/github.com/midoshouse/ootr-multiworld/ --exclude .cargo/config.toml --exclude target --exclude crate/multiworld-bizhawk/OotrMultiworld/BizHawk --exclude crate/multiworld-bizhawk/OotrMultiworld/src/bin --exclude crate/multiworld-bizhawk/OotrMultiworld/src/obj --exclude crate/multiworld-bizhawk/OotrMultiworld/src/multiworld.dll
ThrowOnNativeFailure
debian run env -C /home/fenhl/wslgit/github.com/midoshouse/ootr-multiworld assets/bootstrap-linux.sh
ThrowOnNativeFailure

#TODO move to release script
debian run mkdir -p /mnt/c/Users/fenhl/git/github.com/midoshouse/ootr-multiworld/stage/target/wsl/release
debian run cp /home/fenhl/wslgit/github.com/midoshouse/ootr-multiworld/target/release/multiworld-gui /mnt/c/Users/fenhl/git/github.com/midoshouse/ootr-multiworld/stage/target/wsl/release/multiworld-gui
ThrowOnNativeFailure
