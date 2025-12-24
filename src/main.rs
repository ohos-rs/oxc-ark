use owo_colors::OwoColorize;

use crate::cli::cli_run;

mod cli;
mod format;

#[derive(Debug, Clone)]
pub(crate) struct FormatArgs {
    file: Vec<String>,
    thread: usize,
    excludes: Vec<String>,
    // FormatOptions fields (excluding quote_properties)
    pub indent_style: Option<oxc_formatter::IndentStyle>,
    pub indent_width: Option<oxc_formatter::IndentWidth>,
    pub line_ending: Option<oxc_formatter::LineEnding>,
    pub line_width: Option<oxc_formatter::LineWidth>,
    pub quote_style: Option<oxc_formatter::QuoteStyle>,
    pub jsx_quote_style: Option<oxc_formatter::QuoteStyle>,
    pub trailing_commas: Option<oxc_formatter::TrailingCommas>,
    pub semicolons: Option<oxc_formatter::Semicolons>,
    pub arrow_parentheses: Option<oxc_formatter::ArrowParentheses>,
    pub bracket_spacing: Option<oxc_formatter::BracketSpacing>,
    pub bracket_same_line: Option<oxc_formatter::BracketSameLine>,
    pub attribute_position: Option<oxc_formatter::AttributePosition>,
    pub expand: Option<oxc_formatter::Expand>,
    pub experimental_operator_position: Option<oxc_formatter::OperatorPosition>,
    pub experimental_ternaries: Option<bool>,
    pub embedded_language_formatting: Option<oxc_formatter::EmbeddedLanguageFormatting>,
    #[allow(dead_code)]
    pub experimental_sort_imports: Option<String>, // JSON string for SortImportsOptions (not yet implemented)
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
