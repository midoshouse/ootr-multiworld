function ThrowOnNativeFailure {
    if (-not $?)
    {
        throw 'Native Failure'
    }
}

"copying Linux artifacts to peterpc3"
scp .\target\wsl\release\libmultiworld.so 192.168.178.77:bin/BizHawk/dll
ThrowOnNativeFailure
scp .\target\wsl\release\OotrMultiworld.dll 192.168.178.77:bin/BizHawk/ExternalTools
ThrowOnNativeFailure
