use bpaf::{Parser, construct, long, positional};
use std::str::FromStr;

pub fn cli_format() -> impl Parser<crate::Options> {
    let file = positional("input")
        .help("Input regex to select files.")
        .many();

    let thread = long("thread")
        .short('t')
        .argument("THREAD")
        .help("Thread count for parallel formatting.")
        .fallback(1);

    let excludes = long("exclude")
        .argument("PATTERN")
        .help("Exclude files or directories.")
        .many()
        .fallback(vec![]);

    // FormatOptions parameters
    let indent_style = long("indent-style")
        .argument::<String>("STYLE")
        .help("The indent style. Values: tab, space")
        .parse(|s| oxc_formatter::IndentStyle::from_str(&s))
        .optional();

    let indent_width = long("indent-width")
        .argument::<String>("WIDTH")
        .help("The indent width (0-24)")
        .parse(|s| oxc_formatter::IndentWidth::from_str(&s))
        .optional();

    let line_ending = long("line-ending")
        .argument::<String>("ENDING")
        .help("The type of line ending. Values: lf, crlf, cr")
        .parse(|s| oxc_formatter::LineEnding::from_str(&s))
        .optional();

    let line_width = long("line-width")
        .argument::<String>("WIDTH")
        .help("The max width of a line (1-320)")
        .parse(|s| oxc_formatter::LineWidth::from_str(&s))
        .optional();

    let quote_style = long("quote-style")
        .argument::<String>("STYLE")
        .help("The style for quotes. Values: double, single")
        .parse(|s| oxc_formatter::QuoteStyle::from_str(&s))
        .optional();

    let jsx_quote_style = long("jsx-quote-style")
        .argument::<String>("STYLE")
        .help("The style for JSX quotes. Values: double, single")
        .parse(|s| oxc_formatter::QuoteStyle::from_str(&s))
        .optional();

    let trailing_commas = long("trailing-commas")
        .argument::<String>("VALUE")
        .help("Print trailing commas. Values: all, es5, none")
        .parse(|s| oxc_formatter::TrailingCommas::from_str(&s))
        .optional();

    let semicolons = long("semicolons")
        .argument::<String>("VALUE")
        .help("Print semicolons. Values: always, as-needed")
        .parse(|s| oxc_formatter::Semicolons::from_str(&s))
        .optional();

    let arrow_parentheses = long("arrow-parentheses")
        .argument::<String>("VALUE")
        .help("Add parentheses to arrow functions. Values: always, as-needed")
        .parse(|s| oxc_formatter::ArrowParentheses::from_str(&s))
        .optional();

    let bracket_spacing = long("bracket-spacing")
        .argument::<String>("VALUE")
        .help("Insert spaces around brackets in object literals. Values: true, false")
        .parse(|s| oxc_formatter::BracketSpacing::from_str(&s))
        .optional();

    let bracket_same_line = long("bracket-same-line")
        .argument::<String>("VALUE")
        .help("Hug closing bracket of multiline HTML/JSX tags. Values: true, false")
        .parse(|s| oxc_formatter::BracketSameLine::from_str(&s))
        .optional();

    let attribute_position = long("attribute-position")
        .argument::<String>("VALUE")
        .help("Attribute position style. Values: auto, multiline")
        .parse(|s| oxc_formatter::AttributePosition::from_str(&s))
        .optional();

    let expand = long("expand")
        .argument::<String>("VALUE")
        .help("Expand object and array literals. Values: auto, always, never")
        .parse(|s| oxc_formatter::Expand::from_str(&s))
        .optional();

    let experimental_operator_position = long("experimental-operator-position")
        .argument::<String>("VALUE")
        .help("Operator position in binary expressions. Values: start, end")
        .parse(|s| oxc_formatter::OperatorPosition::from_str(&s))
        .optional();

    let experimental_ternaries = long("experimental-ternaries")
        .argument::<String>("VALUE")
        .help("Use curious ternaries. Values: true, false")
        .parse(|s| bool::from_str(&s).map_err(|_| "Value must be 'true' or 'false'"))
        .optional();

    let embedded_language_formatting = long("embedded-language-formatting")
        .argument::<String>("VALUE")
        .help("Enable formatting for embedded languages. Values: auto, off")
        .parse(|s| oxc_formatter::EmbeddedLanguageFormatting::from_str(&s))
        .optional();

    let experimental_sort_imports = long("experimental-sort-imports")
        .argument("JSON")
        .help("Sort import statements. Provide JSON configuration string")
        .optional();

    let format_parser = construct!(crate::FormatArgs {
        thread,
        excludes,
        indent_style,
        indent_width,
        line_ending,
        line_width,
        quote_style,
        jsx_quote_style,
        trailing_commas,
        semicolons,
        arrow_parentheses,
        bracket_spacing,
        bracket_same_line,
        attribute_position,
        expand,
        experimental_operator_position,
        experimental_ternaries,
        embedded_language_formatting,
        experimental_sort_imports,
        file,
    });
    construct!(crate::Options::Format(format_parser))
}
