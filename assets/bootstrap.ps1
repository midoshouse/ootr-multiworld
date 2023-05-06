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
"installing prerequisite packages on Linux"
wsl sudo apt-get install -y cmake dotnet-sdk-6.0 libfontconfig1-dev libfreetype6-dev libssl-dev pkg-config python3 rsync
ThrowOnNativeFailure
"creating target dir on Linux"
wsl mkdir -p /home/fenhl/wslgit/github.com/midoshouse/ootr-multiworld/target/debug
ThrowOnNativeFailure
"syncing repo to Linux"
wsl rsync --delete -av /mnt/c/Users/fenhl/git/github.com/midoshouse/ootr-multiworld/stage/ /home/fenhl/wslgit/github.com/midoshouse/ootr-multiworld/ --exclude .cargo/config.toml --exclude target --exclude crate/multiworld-bizhawk/OotrMultiworld/BizHawk --exclude crate/multiworld-bizhawk/OotrMultiworld/src/bin --exclude crate/multiworld-bizhawk/OotrMultiworld/src/obj --exclude crate/multiworld-bizhawk/OotrMultiworld/src/multiworld.dll
ThrowOnNativeFailure
"running bootstrap-linux.sh on Linux"
wsl env -C /home/fenhl/wslgit/github.com/midoshouse/ootr-multiworld assets/bootstrap-linux.sh
ThrowOnNativeFailure

#TODO move to release script
"creating WSL target dir"
wsl mkdir -p /mnt/c/Users/fenhl/git/github.com/midoshouse/ootr-multiworld/stage/target/wsl/release
ThrowOnNativeFailure
"copying Linux artifacts to Windows file system"
wsl cp /home/fenhl/wslgit/github.com/midoshouse/ootr-multiworld/target/release/multiworld-gui /mnt/c/Users/fenhl/git/github.com/midoshouse/ootr-multiworld/stage/target/wsl/release/multiworld-gui
ThrowOnNativeFailure
wsl cp /home/fenhl/wslgit/github.com/midoshouse/ootr-multiworld/crate/multiworld-bizhawk/OotrMultiworld/BizHawk/dll/libmultiworld.so /mnt/c/Users/fenhl/git/github.com/midoshouse/ootr-multiworld/stage/target/wsl/release/libmultiworld.so
ThrowOnNativeFailure
wsl cp /home/fenhl/wslgit/github.com/midoshouse/ootr-multiworld/crate/multiworld-bizhawk/OotrMultiworld/BizHawk/ExternalTools/OotrMultiworld.dll /mnt/c/Users/fenhl/git/github.com/midoshouse/ootr-multiworld/stage/target/wsl/release/OotrMultiworld.dll
ThrowOnNativeFailure

#TODO move to a testing script?
"copying Linux artifacts to peterpc3"
scp .\target\wsl\release\libmultiworld.so 192.168.178.77:bin/BizHawk/dll
ThrowOnNativeFailure
scp .\target\wsl\release\OotrMultiworld.dll 192.168.178.77:bin/BizHawk/ExternalTools
ThrowOnNativeFailure

"bootstrap done"
