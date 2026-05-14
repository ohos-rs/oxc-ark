use std::ffi::OsString;

pub fn lint(args: Vec<OsString>) -> bool {
    lint::lint_args(args)
}
