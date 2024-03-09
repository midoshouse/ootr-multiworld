function ThrowOnNativeFailure {
    if (-not $?) {
        throw 'Native Failure'
    }
}

cargo build --package=multiworld-gui
ThrowOnNativeFailure

Copy-Item -Path .\target\debug\multiworld-gui.exe -Destination 'C:\Users\fenhl\AppData\Local\Fenhl\OoTR Multiworld\cache\gui.exe'

Copy-Item -Path .\assets\ootrmw-pj64.js -Destination 'C:\Program Files (x86)\Project64 3.0\Scripts\ootrmw-dev.js'

Set-Location 'C:\Program Files (x86)\Project64 3.0\'

.\Project64.exe

cargo run --package=multiworld-gui
ThrowOnNativeFailure
