function ThrowOnNativeFailure {
    if (-not $?) {
        throw 'Native Failure'
    }
}

git push
ThrowOnNativeFailure

cargo run --release --package=multiworld-release -- @args
ThrowOnNativeFailure
