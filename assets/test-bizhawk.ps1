function ThrowOnNativeFailure {
    if (-not $?) {
        throw 'Native Failure'
    }
}

#TODO make sure BizHawk is up to date

cargo build --package=multiworld-csharp
ThrowOnNativeFailure

cargo build --package=multiworld-bizhawk
ThrowOnNativeFailure

Set-Location .\crate\multiworld-bizhawk\OotrMultiworld\BizHawk
.\EmuHawk.exe --open-ext-tool-dll=OotrMultiworld
Set-Location ..\..\..\..
