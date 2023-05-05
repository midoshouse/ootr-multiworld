# runs build commands that may be required by other build commands (since some crates include code from other crates, e.g. updaters)

function ThrowOnNativeFailure {
    if (-not $?)
    {
        throw 'Native Failure'
    }
}

# multiworld-updater
"building multiworld-updater (debug)"
cargo build --package=multiworld-updater
ThrowOnNativeFailure
"building multiworld-updater (release)"
cargo build --release --package=multiworld-updater
ThrowOnNativeFailure

# multiworld-gui
"building multiworld-gui (debug)"
cargo build --package=multiworld-gui
ThrowOnNativeFailure
"building multiworld-gui (release)"
cargo build --release --package=multiworld-gui
ThrowOnNativeFailure

# multiworld-csharp
"building multiworld-csharp (debug)"
cargo build --package=multiworld-csharp
ThrowOnNativeFailure
"building multiworld-csharp (release)"
cargo build --release --package=multiworld-csharp
ThrowOnNativeFailure

# multiworld-bizhawk
"building multiworld-bizhawk (debug)"
cargo build --package=multiworld-bizhawk
ThrowOnNativeFailure
"building multiworld-bizhawk (release)"
cargo build --release --package=multiworld-bizhawk
ThrowOnNativeFailure

# multiworld-installer
"building multiworld-installer (debug)"
cargo build --package=multiworld-installer
ThrowOnNativeFailure
"building multiworld-installer (release)"
cargo build --release --package=multiworld-installer
ThrowOnNativeFailure

# Linux
# build on Debian because Ubuntu's glibc is too new for compatibility with Debian
"installing prerequisite packages on Debian"
debian run sudo apt-get install -y cmake libfontconfig-dev libssl-dev pkg-config python3 rsync
"creating target dir on Debian"
debian run mkdir -p /home/fenhl/wslgit/github.com/midoshouse/ootr-multiworld/target/debug
"syncing repo to Debian"
debian run rsync --delete -av /mnt/c/Users/fenhl/git/github.com/midoshouse/ootr-multiworld/stage/ /home/fenhl/wslgit/github.com/midoshouse/ootr-multiworld/ --exclude .cargo/config.toml --exclude target --exclude crate/multiworld-bizhawk/OotrMultiworld/BizHawk --exclude crate/multiworld-bizhawk/OotrMultiworld/src/bin --exclude crate/multiworld-bizhawk/OotrMultiworld/src/obj --exclude crate/multiworld-bizhawk/OotrMultiworld/src/multiworld.dll
ThrowOnNativeFailure
"running bootstrap-linux.sh on Debian"
debian run env -C /home/fenhl/wslgit/github.com/midoshouse/ootr-multiworld assets/bootstrap-linux.sh
ThrowOnNativeFailure

#TODO move to release script
"creating WSL target dir"
debian run mkdir -p /mnt/c/Users/fenhl/git/github.com/midoshouse/ootr-multiworld/stage/target/wsl/release
"copying Linux artifacts to Windows file system"
debian run cp /home/fenhl/wslgit/github.com/midoshouse/ootr-multiworld/target/release/multiworld-gui /mnt/c/Users/fenhl/git/github.com/midoshouse/ootr-multiworld/stage/target/wsl/release/multiworld-gui
ThrowOnNativeFailure
"bootstrap done"
