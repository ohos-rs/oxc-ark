use owo_colors::OwoColorize;

use crate::cli::cli_run;

mod cli;
mod format;

#[derive(Debug, Clone)]
pub(crate) struct FormatArgs {
    file: Vec<String>,
    thread: usize,
    excludes: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) enum Options {
    Format(FormatArgs),
}

fn main() {
    let parser = cli_run()
        .descr(cli::Info())
        .version(env!("CARGO_PKG_VERSION"));

    let ret = parser.fallback_to_usage().run();

    let run_ret = match ret {
        Options::Format(args) => format::format(args),
    };
    if let Err(e) = run_ret {
        println!("{:?}", e.red());
        std::process::exit(-1);
    }
}
