use owo_colors::OwoColorize;
use std::{ffi::OsStr, ffi::OsString};

use crate::cli::cli_run;

mod cli;
mod format;
mod lint_cmd;

#[derive(Debug, Clone)]
pub(crate) struct FormatArgs {
    pub lsp: bool,
    pub config: Option<std::path::PathBuf>,
    file: Vec<String>,
    thread: usize,
    excludes: Vec<String>,
    ignore_path: Vec<std::path::PathBuf>,
    with_node_modules: bool,
    // FormatOptions fields (excluding quote_properties)
    pub indent_style: Option<oxc_formatter_core::IndentStyle>,
    pub indent_width: Option<oxc_formatter_core::IndentWidth>,
    pub line_ending: Option<oxc_formatter_core::LineEnding>,
    pub line_width: Option<oxc_formatter_core::LineWidth>,
    pub quote_style: Option<oxc_formatter::QuoteStyle>,
    pub jsx_quote_style: Option<oxc_formatter::QuoteStyle>,
    pub trailing_commas: Option<oxc_formatter::TrailingCommas>,
    pub semicolons: Option<oxc_formatter::Semicolons>,
    pub arrow_parentheses: Option<oxc_formatter::ArrowParentheses>,
    pub bracket_spacing: Option<oxc_formatter::BracketSpacing>,
    pub bracket_same_line: Option<oxc_formatter::BracketSameLine>,
    pub attribute_position: Option<oxc_formatter::AttributePosition>,
    pub expand: Option<oxc_formatter::Expand>,
    #[allow(dead_code)]
    // reserved for CLI/oxfmt compatibility; oxfmtrc rejects these in into_oxfmt_options
    pub experimental_operator_position: Option<oxc_formatter::OperatorPosition>,
    #[allow(dead_code)]
    pub experimental_ternaries: Option<bool>,
    pub embedded_language_formatting: Option<oxc_formatter::EmbeddedLanguageFormatting>,
    #[allow(dead_code)]
    pub experimental_sort_imports: Option<String>, // JSON string for SortImportsOptions (not yet implemented)
}

#[derive(Debug, Clone)]
pub(crate) enum Options {
    Format(FormatArgs),
    Lint(Vec<OsString>),
}

fn main() {
    let mut raw_args = std::env::args_os();
    let _bin = raw_args.next();
    if raw_args.next().is_some_and(|arg| arg == OsStr::new("lint")) {
        let success = lint_cmd::lint(std::env::args_os().skip(2).collect());
        if !success {
            std::process::exit(1);
        }
        return;
    }

    let parser = cli_run()
        .descr(cli::Info())
        .version(env!("CARGO_PKG_VERSION"));

    let ret = parser.fallback_to_usage().run();

    let run_ret: Result<(), Box<dyn std::error::Error>> = match ret {
        Options::Format(args) => {
            if args.lsp {
                format::run_lsp()
            } else {
                format::format(args)
            }
        }
        Options::Lint(args) => {
            if lint_cmd::lint(args) {
                Ok(())
            } else {
                Err(Box::new(std::io::Error::other("lint failed")) as Box<dyn std::error::Error>)
            }
        }
    };
    if let Err(e) = run_ret {
        println!("{:?}", e.red());
        std::process::exit(-1);
    }
}
