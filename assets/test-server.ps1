function ThrowOnNativeFailure {
    if (-not $?) {
        throw 'Native Failure'
    }
}

debian run rsync --delete -av /mnt/c/Users/fenhl/git/github.com/midoshouse/ootr-multiworld/stage/ /home/fenhl/wslgit/github.com/midoshouse/ootr-multiworld/ --exclude .cargo/config.toml --exclude target --exclude crate/multiworld-bizhawk/OotrMultiworld/BizHawk --exclude crate/multiworld-bizhawk/OotrMultiworld/src/bin --exclude crate/multiworld-bizhawk/OotrMultiworld/src/obj --exclude crate/multiworld-bizhawk/OotrMultiworld/src/multiworld.dll
ThrowOnNativeFailure

debian run env -C /home/fenhl/wslgit/github.com/midoshouse/ootr-multiworld /home/fenhl/.cargo/bin/cargo build --package=ootrmwd
ThrowOnNativeFailure

debian run cp /home/fenhl/wslgit/github.com/midoshouse/ootr-multiworld/target/debug/ootrmwd /mnt/c/Users/fenhl/git/github.com/midoshouse/ootr-multiworld/stage/target/wsl/debug/ootrmwd
ThrowOnNativeFailure

scp target/wsl/debug/ootrmwd midos.house:bin/ootrmwd-debug
ThrowOnNativeFailure

ssh midos.house chmod +x bin/ootrmwd-debug
ThrowOnNativeFailure

ssh midos.house sudo -u mido bin/ootrmwd-debug --port=18824 --database=ootr_multiworld_dev
ThrowOnNativeFailure
