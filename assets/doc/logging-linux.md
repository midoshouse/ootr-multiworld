# Enabling logging

If you are asked to enable logging to help with debugging, please follow these steps:

1. Open the file `~/.config/midos-house/multiworld.json` in a text editor, creating it if it doesn't exist. (Note: If you have set the `$XDG_CONFIG_HOME` and/or `$XDG_CONFIG_DIRS` environment variables, the location may differ. See [the XDG Base Directory Specification](https://specifications.freedesktop.org/basedir-spec/basedir-spec-latest.html) for details.)
2. If the file already existed, look for an entry like `"log": false` and change it to `"log": true`. If there is no `"log"` entry, add it. Let's say the file has contents similar to this:
    ```json
    {
        "default_frontend": "BizHawk"
    }
    ```
    In this case, add the `"log"` entry like this:
    ```json
    {
        "default_frontend": "BizHawk",
        "log": true
    }
    ```
    Make sure to include the `,` between the existing entries and the `"log"` entry. If the file did not exist, enter the following text:
    ```json
    {"log": true}
    ```
3. Save and close `config.json`.
4. Completely close Mido's House Multiworld, as well as BizHawk if you're using BizHawk.

# Locating the log files

The log files have the `.log` file extension and will be in `~/.local/share/midos-house` by default. (Note: If you have set the `$XDG_CONFIG_HOME` and/or `$XDG_CONFIG_DIRS` environment variables, the location may differ. See [the XDG Base Directory Specification](https://specifications.freedesktop.org/basedir-spec/basedir-spec-latest.html) for details.) Please send the one that was requested, or all of them if you're not sure. **Warning:** These files may contain sensitive information such as room passwords or your Linux username.

# Disabling logging

Once you have provided the logs, you can disable logging again to improve performance and save disk space:

1. Edit the `multiworld.json` config file to replace `"log": true` with `"log": false`, then save and close `multiworld.json`.
2. Completely close Mido's House Multiworld, as well as BizHawk if you're using BizHawk.
3. Optionally, delete the existing log files.
