use std::ffi::OsString;

use bpaf::{Parser, any, construct};

pub fn cli_lint() -> impl Parser<crate::Options> {
    let args = any::<OsString, _, _>("ARGS", |arg| {
        (arg != "--help" && arg != "-h").then_some(arg)
    })
    .help("Arguments passed to oxlint-compatible lint runner.")
    .many();

    construct!(crate::Options::Lint(args))
}
