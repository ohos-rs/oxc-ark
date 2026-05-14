// Portions of this file are derived from Oxc's oxfmt implementation.
// Copyright (c) Oxc project contributors.
// Licensed under the MIT License. See https://github.com/oxc-project/oxc/blob/main/LICENSE.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

/// Normalize a relative path by stripping `./` prefix and joining with `cwd`.
/// This ensures consistent path format and avoids issues with relative paths.
/// Aligned with oxfmt's `utils::normalize_relative_path`.
pub fn normalize_relative_path(cwd: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    if let Ok(stripped) = path.strip_prefix("./") {
        cwd.join(stripped)
    } else {
        cwd.join(path)
    }
}

pub fn read_to_string(path: &Path) -> io::Result<String> {
    // `simdutf8` is faster than `std::str::from_utf8` which `fs::read_to_string` uses internally
    let bytes = fs::read(path)?;
    if simdutf8::basic::from_utf8(&bytes).is_err() {
        // Same error as `fs::read_to_string` produces (using `io::ErrorKind::InvalidData`)
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "stream did not contain valid UTF-8",
        ));
    }
    // SAFETY: `simdutf8` has ensured it's a valid UTF-8 string
    Ok(unsafe { String::from_utf8_unchecked(bytes) })
}
