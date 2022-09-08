This is an alternative implementation of [multiworld](https://wiki.ootrandomizer.com/index.php?title=Multiworld) for [the Ocarina of Time randomizer](https://ootrandomizer.com/) that improves upon [the existing implementation](https://github.com/TestRunnerSRL/bizhawk-co-op) by breaking compatibility with it.

# Installing

## Automatic (recommended)

The easiest way to set everything up is by downloading and running [the installer](https://github.com/midoshouse/ootr-multiworld/releases/latest/download/multiworld-installer.exe). It will guide you through setting up multiworld for BizHawk or Project64, and will also offer to install the emulator if you don't have it yet.

## Manual

If you can't or don't want to use the installer, you can follow the manual install instructions:

* For BizHawk:
    1. Download and run [BizHawk-Prereqs](https://github.com/TASEmulators/BizHawk-Prereqs/releases/latest).
    2. Download [BizHawk](https://github.com/TASEmulators/BizHawk/releases/latest) and extract it to somewhere you'll find it again.
    3. Open the extracted BizHawk folder (the one with `EmuHawk.exe` in it). If there's no folder named `ExternalTools` inside it, create one.
    4. Download [multiworld for BizHawk](https://github.com/midoshouse/ootr-multiworld/releases/latest/download/multiworld-bizhawk.zip).
    5. Open the downloaded archive and move the files `OotrMultiworld.dll` and `multiworld.dll` into the `ExternalTools` folder.
    6. Open BizHawk (`EmuHawk.exe`).
    7. In BizHawk, go to Tools menu → External Tool → Mido's House Multiworld.
    8. A window should open that lets you connect to or create a room. Keep this window open during your seed (you can minimize it if you want).
* For Project64:
    1. Download, install, and run [Project64](https://www.pj64-emu.com/).
    2. In Project64's Options menu, select Configuration.
    3. In General settings, uncheck the “Hide advanced settings” setting.
    4. In Advanced, check the “Enable debugger” setting.
    5. Click OK.
    6. Download [the multiworld companion app for Project64](https://github.com/midoshouse/ootr-multiworld/releases/latest/download/multiworld-pj64.exe), put it somewhere you'll find it again, and open it.
    7. In Project64's Debugger menu, select Scripts.
    8. Click the “…” button. A File Explorer window should open, showing the Scripts subfolder of your Project64 installation. If it shows a different folder, navigate to the Scripts folder (`C:\Program Files (x86)\Project64 3.0\Scripts` by default). You may have to create that folder.
    9. Download [the multiworld script for Project64](https://github.com/midoshouse/ootr-multiworld/releases/latest/download/ootrmw-pj64.js), put it into the Scripts folder, and rename it to `ootrmw.js`. You can then close the File Explorer window.
    10. In Project64's Scripts window, select `ootrmw.js` and click Run. You can then close the Scripts window.
    11. The companion app should now allow you to connect to or create a room. Keep the companion app open during your seed (you can minimize it if you want).

If you need help, please ask in #setup-support on the OoTR Discord.

# Feature comparison

|Feature|[bizhawk-co-op](https://github.com/TestRunnerSRL/bizhawk-co-op)|Mido's House Multiworld|
|---|---|---|
|[Project64](https://pj64-emu.com/) support||✓|
|support for older versions of BizHawk|✓||
|better performance on BizHawk||✓|
|no port forwarding or Hamachi required||✓|
|can be used via LAN without an internet connection|✓|planned ([#3](https://github.com/midoshouse/ootr-multiworld/issues/3))|
|easier setup: player name and world number are read from the game||✓|
|prevents players from accidentally using the same world number||✓|
|support for some other games|✓||
|automatically updates itself||✓|

# Credits

* Icon based on chest image created for [Mido's House](https://midos.house/) by [Maplestar](https://github.com/Maplesstar).
* Some code based on [Bizhawk Shuffler 2](https://github.com/authorblues/bizhawk-shuffler-2)
