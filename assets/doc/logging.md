# Enabling logging

If you are asked to enable logging to help with debugging, please follow these steps:

1. Press <kbd>Windows</kbd><kbd>R</kbd>, enter `%APPDATA%`, and click OK.
2. In the File Explorer window that opens, navigate to the following nested subfolder, creating any folders that don't exist:
    * `Fenhl`
        * `OoTR Multiworld`
            * `config`
3. Make sure “View → Show → File name extensions” is checked.
4. In the `config` folder, check if there is a file named `config.json`. If there is not, create a new text document and rename it to `config.json`, confirming if warned about changing the file extension.
5. Open `config.json` with Notepad. If the file already existed, look for an entry like `"log": false` and change it to `"log": true`. If there is no `"log"` entry, add it. Let's say the file has contents similar to this:
    ```json
    {
        "pj64_script_path": "D:\\My Programs\\PJ64\\Scripts"
    }
    ```
    In this case, add the `"log"` entry like this:
    ```json
    {
        "pj64_script_path": "D:\\My Programs\\PJ64\\Scripts",
        "log": true
    }
    ```
    Make sure to include the `,` between the existing entries and the `"log"` entry. If the file did not exist, enter the following text:
    ```json
    {"log": true}
    ```
6. Save and close `config.json`. You can also close the File Explorer window.
7. Completely close Mido's House Multiworld, as well as BizHawk if you're using BizHawk.

# Locating the log files

After reproducing the issue, follow these steps to get the logs:

1. Press <kbd>Windows</kbd><kbd>R</kbd>, enter `%APPDATA%`, and click OK.
2. In the File Explorer window that opens, navigate to the following nested subfolder:
    * `Fenhl`
        * `OoTR Multiworld`
            * `data`
3. There should be one or more files with the `.log` file extension (e.g. `ffi.log`, `gui.log`, and/or `updater.log`) in this folder. Please send the one that was requested, or all of them if you're not sure. **Warning:** These files may contain sensitive information such as room passwords or your Windows username.

# Disabling logging

Once you have provided the logs, you can disable logging again to improve performance and save disk space:

1. Edit `config.json` to replace `"log": true` with `"log": false`, then save and close `config.json`.
2. Completely close Mido's House Multiworld, as well as BizHawk if you're using BizHawk.
3. Optionally, delete the existing log files.
