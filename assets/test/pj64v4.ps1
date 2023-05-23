function ThrowOnNativeFailure {
    if (-not $?) {
        throw 'Native Failure'
    }
}

cargo build --package=multiworld-gui
ThrowOnNativeFailure

Copy-Item -Path .\target\debug\multiworld-gui.exe -Destination 'C:\Users\fenhl\AppData\Local\Fenhl\OoTR Multiworld\cache\gui.exe'

Copy-Item -Path .\assets\ootrmw-pj64v4.js -Destination 'C:\Users\fenhl\bin\Project64-Dev-4.0.0-6097-2c40d47\Scripts\ootrmw.js'

& 'C:\Users\fenhl\bin\Project64-Dev-4.0.0-6097-2c40d47\Project64.exe'
