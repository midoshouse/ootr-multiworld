function ThrowOnNativeFailure {
    if (-not $?) {
        throw 'Native Failure'
    }
}

cargo run --release --package=multiworld-utils --bin=multiworld-release -- @args
ThrowOnNativeFailure
