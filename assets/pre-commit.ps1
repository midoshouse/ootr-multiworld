#!/usr/bin/env pwsh

function ThrowOnNativeFailure {
    if (-not $?)
    {
        throw 'Native Failure'
    }
}

cargo check --package=multiworld # for checking without sqlx feature
ThrowOnNativeFailure

cargo check --workspace
ThrowOnNativeFailure

# copy the tree to the WSL file system to improve compile times
wsl rsync --delete -av /mnt/c/Users/fenhl/git/github.com/midoshouse/ootr-multiworld/stage/ /home/fenhl/wslgit/github.com/midoshouse/ootr-multiworld/ --exclude .cargo/config.toml --exclude target --exclude crate/multiworld-bizhawk/OotrMultiworld/BizHawk --exclude crate/multiworld-bizhawk/OotrMultiworld/src/bin --exclude crate/multiworld-bizhawk/OotrMultiworld/src/obj --exclude crate/multiworld-bizhawk/OotrMultiworld/src/multiworld.dll
ThrowOnNativeFailure

wsl env -C /home/fenhl/wslgit/github.com/midoshouse/ootr-multiworld cargo check --package=ootrmwd
ThrowOnNativeFailure
