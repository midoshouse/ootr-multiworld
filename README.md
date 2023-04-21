This is **Mido's House Multiworld**, an alternative implementation of [multiworld](https://wiki.ootrandomizer.com/index.php?title=Multiworld) for [the Ocarina of Time randomizer](https://ootrandomizer.com/) that improves upon [the existing implementation](https://github.com/TestRunnerSRL/bizhawk-co-op) by breaking compatibility with it.

# Installing

The easiest and recommended way to set everything up is by downloading and running [the installer](https://github.com/midoshouse/ootr-multiworld/releases/latest/download/multiworld-installer.exe). It will guide you through setting up multiworld for BizHawk or Project64, and will also offer to install the emulator if you don't have it yet.

If you need help, please ask in [#setup-support on the OoTR Discord](https://discord.gg/BGRrKKn) (feel free to ping @Fenhl#4813) or [open an issue](https://github.com/midoshouse/ootr-multiworld/issues/new).

If you can't use the installer due to antivirus software blocking it, you can follow [the manual install instructions](https://github.com/midoshouse/ootr-multiworld/blob/main/assets/doc/manual-install.md).

# Feature comparison

|Feature|[bizhawk-co-op](https://github.com/TestRunnerSRL/bizhawk-co-op)|Mido's House Multiworld|
|---|---|---|
|[Project64](https://pj64-emu.com/) support||✓|
|[BizHawk](https://tasvideos.org/BizHawk) support|2.3–2.8 (no support for the current version)|2.9 only (no support for older versions)|
|no port forwarding or Hamachi required||✓|
|async support: players don't need to be connected at the same time||✓|
|can be used via LAN without an internet connection|✓|planned ([#3](https://github.com/midoshouse/ootr-multiworld/issues/3))|
|easier setup: player name and world number are read from the game||✓|
|prevents players from accidentally using the same world number||✓|
|support for some other games|✓||
|automatically updates itself||✓|
|send all remaining items from a world using a spoiler log|using [an external service](https://pidgezero.one/zootr/mwlua.html)|built in|
|choose individual items to give to a player|using [an external service](https://pidgezero.one/zootr/mwlua.html)||

# Credits

* Icon based on chest image created for [Mido's House](https://midos.house/) by [Maplestar](https://github.com/Maplesstar).
* Some seed hash icons by [Xopar](https://github.com/matthewkirby) and shiroaeli, used with permission.
* Some code based on [Bizhawk Shuffler 2](https://github.com/authorblues/bizhawk-shuffler-2)
