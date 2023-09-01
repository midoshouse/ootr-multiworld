# Debugging a crash

If the multiworld installer or app unexpectedly closes without showing an error message, follow these instructions to view the error message:

1. Depending on whether it is the installer or the multiworld app itself that is crashing, download the appropriate file from the list below but don't open it.
    * [Download this if the installer is crashing](https://github.com/midoshouse/ootr-multiworld/releases/latest/download/multiworld-installer-debug.exe)
    * [Download this if the multiworld app itself is crashing](https://github.com/midoshouse/ootr-multiworld/releases/latest/download/multiworld-gui-debug.exe)
2. Right-click your Windows button (the one that opens the Start menu) and select “Terminal” or “Windows PowerShell” depending on which one you have.
3. Open the folder with the downloaded file in it in File Explorer, right-click the file, and select “Copy as path”.
    * On older versions of Windows (up to Windows 10), hold <kbd>⇧ Shift</kbd> while right-clicking to show the “Copy as path” option.
4. Go back to Terminal/PowerShell, type `&` and a space, then right-click on the Terminal/PowerShell window (which should paste the path).
5. Press <kbd>Return ⏎</kbd>. An error message should appear in the Terminal/PowerShell window. Send that error message to [#setup-support on the OoTR Discord](https://discord.gg/BGRrKKn) (feel free to ping `@fenhl`) or [open an issue](https://github.com/midoshouse/ootr-multiworld/issues/new) for it.
