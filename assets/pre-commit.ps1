#!/usr/bin/env pwsh

cargo check --package=multiworld # for checking without sqlx feature
if (-not $?)
{
    throw 'Native Failure'
}

cargo check --workspace
if (-not $?)
{
    throw 'Native Failure'
}

cargo sqlx prepare --workspace --check
if (-not $?)
{
    throw 'Native Failure'
}

wsl rustup update stable
if (-not $?)
{
    throw 'Native Failure'
}

# copy the tree to the WSL file system to improve compile times
wsl rsync --delete -av /mnt/c/Users/fenhl/git/github.com/midoshouse/ootr-multiworld/stage/ /home/fenhl/wslgit/github.com/midoshouse/ootr-multiworld/ --exclude .cargo/config.toml --exclude target --exclude crate/multiworld-bizhawk/OotrMultiworld/BizHawk --exclude crate/multiworld-bizhawk/OotrMultiworld/src/bin --exclude crate/multiworld-bizhawk/OotrMultiworld/src/obj --exclude crate/multiworld-bizhawk/OotrMultiworld/src/multiworld.dll
if (-not $?)
{
    throw 'Native Failure'
}

wsl env -C /home/fenhl/wslgit/github.com/midoshouse/ootr-multiworld cargo check --workspace --exclude=multiworld-release
if (-not $?)
{
    throw 'Native Failure'
}
