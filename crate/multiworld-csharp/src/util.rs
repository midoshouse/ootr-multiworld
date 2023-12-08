use std::{
    path::{
        Path,
        PathBuf,
    },
};
#[cfg(unix)] use std::fs;
#[cfg(windows)] use {
    std::{
        ffi::OsString,
        io,
        iter,
        os::windows::ffi::{
            OsStrExt as _,
            OsStringExt as _,
        },
    },
    itertools::Itertools as _,
    wheel::traits::IoResultExt as _,
    windows::{
        Win32::Storage::FileSystem::GetFullPathNameW,
        core::PCWSTR,
    },
};

pub(crate) fn absolute_path(path: impl AsRef<Path>) -> wheel::Result<PathBuf> {
    let path = path.as_ref();
    #[cfg(unix)] {
        fs::canonicalize(path)
    }
    #[cfg(windows)] {
        let path_wide = path.as_os_str().encode_wide().chain(iter::once(0)).collect_vec();
        let path_ptr = PCWSTR(path_wide.as_ptr());
        Ok(PathBuf::from(unsafe {
            let mut buf = vec![0; 260];
            let result = GetFullPathNameW(path_ptr, Some(&mut buf), None);
            if result == 0 {
                drop(path_wide);
                return Err(io::Error::last_os_error()).at(path)
            } else if result > u32::try_from(buf.len()).expect("buffer too large") {
                buf = vec![0; result.try_into().expect("path too long")];
                let result = GetFullPathNameW(path_ptr, Some(&mut buf), None);
                drop(path_wide);
                if result == 0 {
                    return Err(io::Error::last_os_error()).at(path)
                } else if result > u32::try_from(buf.len()).expect("buffer too large") {
                    panic!("path too long")
                } else {
                    OsString::from_wide(&buf[0..result.try_into().expect("path too long")])
                }
            } else {
                drop(path_wide);
                OsString::from_wide(&buf[0..result.try_into().expect("path too long")])
            }
        }))
    }
}
