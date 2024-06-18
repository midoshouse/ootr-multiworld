This is **Mido's House Multiworld**, an alternative implementation of [multiworld](https://wiki.ootrandomizer.com/index.php?title=Multiworld) for [the Ocarina of Time randomizer](https://ootrandomizer.com/) that improves upon [the existing implementation](https://github.com/TestRunnerSRL/bizhawk-co-op) by breaking compatibility with it. ([Feature comparison](https://wiki.ootrandomizer.com/index.php?title=Multiworld#Feature_comparison))

# Installing

The easiest and recommended way to set everything up is by running the installer ([download for Windows](https://github.com/midoshouse/ootr-multiworld/releases/latest/download/multiworld-installer.exe) • [download for Linux](https://github.com/midoshouse/ootr-multiworld/releases/latest/download/multiworld-installer-linux)). It will guide you through setting up multiworld for EverDrive, BizHawk, or Project64, and will also offer to install BizHawk or Project64 if you don't have it yet.

If you need help, please ask in [#setup-support on the OoTR Discord](https://discord.gg/BGRrKKn) (feel free to ping `@fenhl`) or [open an issue](https://github.com/midoshouse/ootr-multiworld/issues/new).

If you can't use the installer due to antivirus software blocking it, you can follow [the manual install instructions](https://github.com/midoshouse/ootr-multiworld/blob/main/assets/doc/manual-install.md).

# Credits

* Icon based on chest image created for [Mido's House](https://midos.house/) by [Maplestar](https://github.com/Maplesstar).
* Some seed hash icons by [Xopar](https://github.com/matthewkirby) and shiroaeli, used with permission.
* Some code based on [Bizhawk Shuffler 2](https://github.com/authorblues/bizhawk-shuffler-2)

# Building

To ensure `cargo` check/run/etc commands will work, `assets/bootstrap.ps1` needs to be run once. This is because some of the crates in this project will attempt to include some of the others, so they need to be built in a certain order. The bootstrap script can also be re-run later if you want to ensure that all embedded binaries are up to date. It is not necessary to do this before publishing a new version, since the release script also takes care of this.

For the WSL portion of the bootstrap to succeed, the `.cargo/config.toml` file in WSL needs to contain the following:

```toml
[target.x86_64-unknown-linux-gnu]
rustflags = ["-C", "code-model=medium"]
```
