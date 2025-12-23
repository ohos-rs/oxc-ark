use bpaf::{Parser, construct, long, positional};

pub fn cli_format() -> impl Parser<crate::Options> {
    let file = positional("input").help("Input regex to select files.");

    let thread = long("thread")
        .short('t')
        .argument("THREAD")
        .help("Thread count for parallel formatting.")
        .fallback(1);

    let format_parser = construct!(crate::FormatArgs { thread, file });
    construct!(crate::Options::Format(format_parser))
}
