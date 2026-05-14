mod format;
mod lint;

use bpaf::{Doc, OptionParser, Parser, construct};
use owo_colors::OwoColorize;
use owo_colors::colors::CustomColor;

use format::cli_format;
use lint::cli_lint;

pub fn cli_run() -> OptionParser<crate::Options> {
    let format = cli_format()
        .to_options()
        .command("format")
        .help("Format ArkTS/ArkUI code");
    let lint = cli_lint()
        .to_options()
        .command("lint")
        .help("Lint JavaScript, TypeScript, ArkTS, and ArkUI code");

    construct!([format, lint]).to_options()
}

pub struct Info();

static LOGO: &str = r#"
   ______   ___  __
  / __ \ \ / / |/ /
 | |  | \ V /| ' / 
 | |  | |> < |  <  
 | |__| / . \| . \ 
  \____/_/ \_\_|\_\
                   
                                            
"#;

impl From<Info> for Doc {
    fn from(_value: Info) -> Self {
        let mut doc = Self::default();
        doc.text(
            LOGO.fg::<CustomColor<248, 112, 51>>()
                .bold()
                .to_string()
                .as_str(),
        );
        doc.text(
            "\n \n This command is used for parsing, formatting, and linting ArkTS/ArkUI code."
                .blue()
                .to_string()
                .as_str(),
        );
        doc
    }
}

// make sure cli is ok
#[cfg(test)]
mod test {
    use super::cli_run;

    #[test]
    fn check_options() {
        cli_run().check_invariants(false)
    }
}
