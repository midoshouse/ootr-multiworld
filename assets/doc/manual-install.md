# Manual install instructinos

If you can't use [the installer](https://github.com/midoshouse/ootr-multiworld/releases/latest/download/multiworld-installer.exe) due to antivirus software blocking it, you can follow these manual install instructions.

If you need help, please ask in [#setup-support on the OoTR Discord](https://discord.gg/BGRrKKn) (feel free to ping `@fenhl`) or [open an issue](https://github.com/midoshouse/ootr-multiworld/issues/new).

## For BizHawk:

1. Download and run [BizHawk-Prereqs](https://github.com/TASEmulators/BizHawk-Prereqs/releases/latest).
2. Download [BizHawk](https://github.com/TASEmulators/BizHawk/releases/latest) and extract it to somewhere you'll find it again.
3. Open the extracted BizHawk folder (the one with `EmuHawk.exe` in it). If there's no folder named `ExternalTools` inside it, create one.
4. Download [multiworld for BizHawk](https://github.com/midoshouse/ootr-multiworld/releases/latest/download/multiworld-bizhawk.zip).
5. Open the downloaded archive and move the files `OotrMultiworld.dll` and `multiworld.dll` into the `ExternalTools` folder.
6. Open BizHawk (`EmuHawk.exe`).
7. In BizHawk, go to Tools menu → External Tool → Mido's House Multiworld.
8. A window should open that lets you connect to or create a room. Keep this window open during your seed (you can minimize it if you want).

## For Project64:

**Note:** These instructions apply to Project64 versions 3.x only. Version 4.0 of Project64 (currently in development) is not yet fully supported.

1. Download, install, and run [Project64](https://www.pj64-emu.com/).
2. In Project64's Options menu, select Configuration.
3. In General settings, uncheck the “Hide advanced settings” setting.
4. In Advanced, check the “Enable debugger” setting.
5. Click OK.
6. Download [the multiworld companion app for Project64](https://github.com/midoshouse/ootr-multiworld/releases/latest/download/multiworld-pj64.exe), put it somewhere you'll find it again, and open it.
7. In Project64's Debugger menu, select Scripts.
8. Click the “…” button. A File Explorer window should open, showing the Scripts subfolder of your Project64 installation. If it shows a different folder, navigate to the Scripts folder (`C:\Program Files (x86)\Project64 3.0\Scripts` by default). You may have to create that folder.
9. Download [the multiworld script for Project64](https://github.com/midoshouse/ootr-multiworld/releases/latest/download/ootrmw-pj64.js), put it into the Scripts folder, and rename it to `ootrmw.js`.
10. Click on the empty part of the path bar near the top of the File Explorer window. If it displays exactly `C:\Program Files (x86)\Project64 3.0\Scripts`, you can close the File Explorer window and skip to step 17. Otherwise, copy the path to your clipboard.
11. Press <kbd>Windows</kbd><kbd>R</kbd>, enter `%APPDATA%`, and click OK.
12. In the File Explorer window that opens, navigate to the following nested subfolder, creating any folders that don't exist:
    * `Fenhl`
    * `OoTR Multiworld`
    * `config`
13. In the `config` folder, create a new text document and rename it to `config.json`, confirming if warned about changing the file extension.
14. Open `config.json` with Notepad and enter the following text:
    ```json
    {"pj64_script_path": ""}
    ```
15. Between the quotes near the end, paste the path copied in step 11. Replace every `\` with `\\` so the file ends up looking similar to this example (except with the appropriate path):
    ```json
    {"pj64_script_path": "D:\\My Programs\\PJ64\\Scripts"}
    ```
16. Save and close `config.json`. You can also close both File Explorer windows.
17. In Project64's Scripts window, select `ootrmw.js` and click Run. You can then close the Scripts window.
18. The companion app should now allow you to connect to or create a room. Keep the companion app open during your seed (you can minimize it if you want).
