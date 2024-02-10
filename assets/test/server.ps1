function ThrowOnNativeFailure {
    if (-not $?) {
        throw 'Native Failure'
    }
}

wsl --distribution debian-m2 /home/fenhl/.cargo/bin/rustup update stable
ThrowOnNativeFailure

wsl --distribution debian-m2 env -C /home/fenhl/wslgit /home/fenhl/.cargo/bin/cargo sweep -ir
ThrowOnNativeFailure

wsl --distribution debian-m2 rsync --delete -av /mnt/c/Users/fenhl/git/github.com/midoshouse/ootr-multiworld/stage/ /home/fenhl/wslgit/github.com/midoshouse/ootr-multiworld/ --exclude .cargo/config.toml --exclude target --exclude crate/multiworld-bizhawk/OotrMultiworld/BizHawk --exclude crate/multiworld-bizhawk/OotrMultiworld/src/bin --exclude crate/multiworld-bizhawk/OotrMultiworld/src/obj --exclude crate/multiworld-bizhawk/OotrMultiworld/src/multiworld.dll
ThrowOnNativeFailure

wsl --distribution debian-m2 env -C /home/fenhl/wslgit/github.com/midoshouse/ootr-multiworld /home/fenhl/.cargo/bin/cargo build --package=ootrmwd
ThrowOnNativeFailure

wsl --distribution debian-m2 cp /home/fenhl/wslgit/github.com/midoshouse/ootr-multiworld/target/debug/ootrmwd /mnt/c/Users/fenhl/git/github.com/midoshouse/ootr-multiworld/stage/target/wsl/debug/ootrmwd
ThrowOnNativeFailure

ssh midos.house sudo killall -9 ootrmwd-debug

. C:/Users/fenhl/git/github.com/midoshouse/midos.house/stage/assets/reset-dev-env.ps1

scp target/wsl/debug/ootrmwd midos.house:bin/ootrmwd-debug
ThrowOnNativeFailure

ssh midos.house sudo chown mido:www-data bin/ootrmwd-debug
ThrowOnNativeFailure

ssh midos.house sudo chmod +x bin/ootrmwd-debug
ThrowOnNativeFailure

ssh midos.house sudo mv bin/ootrmwd-debug /usr/local/share/midos-house/bin/ootrmwd-debug
ThrowOnNativeFailure

ssh midos.house sudo -u mido /usr/local/share/midos-house/bin/ootrmwd-debug --port=18824 --database=fados_house
ThrowOnNativeFailure
