// Portions of this file are derived from Oxc's oxfmt implementation.
// Copyright (c) Oxc project contributors.
// Licensed under the MIT License. See https://github.com/oxc-project/oxc/blob/main/LICENSE.

//! Minimal .oxfmtrc / OxfmtOptions implementation compatible with oxfmt.
//! Adapted from ohos-rs/oxc apps/oxfmt for use without the full oxfmt crate.

use serde::Deserialize;
use serde_json::Value;

use oxc_formatter::{
    ArrowParentheses, AttributePosition, BracketSameLine, BracketSpacing,
    EmbeddedLanguageFormatting, Expand, JsFormatOptions as FormatOptions, QuoteProperties,
    QuoteStyle, Semicolons, TrailingCommas,
};
use oxc_formatter_core::{IndentStyle, IndentWidth, LineEnding, LineWidth};
use oxc_toml::Options as TomlFormatterOptions;

/// Configuration options for Oxfmt (.oxfmtrc.json / .oxfmtrc.jsonc).
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Oxfmtrc {
    #[serde(flatten)]
    pub format_config: FormatConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overrides: Option<Vec<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ignore_patterns: Option<Vec<String>>,
}

/// Format-related options from .oxfmtrc (Prettier-compatible keys + Oxfmt extensions).
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FormatConfig {
    pub use_tabs: Option<bool>,
    pub tab_width: Option<u8>,
    pub end_of_line: Option<EndOfLineConfig>,
    pub print_width: Option<u16>,
    pub single_quote: Option<bool>,
    pub jsx_single_quote: Option<bool>,
    pub quote_props: Option<QuotePropsConfig>,
    pub trailing_comma: Option<TrailingCommaConfig>,
    pub semi: Option<bool>,
    pub arrow_parens: Option<ArrowParensConfig>,
    pub bracket_spacing: Option<bool>,
    pub bracket_same_line: Option<bool>,
    pub object_wrap: Option<ObjectWrapConfig>,
    pub single_attribute_per_line: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental_operator_position: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental_ternaries: Option<bool>,
    pub embedded_language_formatting: Option<EmbeddedLanguageFormattingConfig>,
    pub insert_final_newline: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental_sort_imports: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental_tailwindcss: Option<Value>,
    /// `true` (default), `false`, or config object (object is accepted but options ignored in minimal impl).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental_sort_package_json: Option<Value>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EndOfLineConfig {
    Lf,
    Crlf,
    Cr,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum QuotePropsConfig {
    AsNeeded,
    Consistent,
    Preserve,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrailingCommaConfig {
    All,
    Es5,
    None,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArrowParensConfig {
    Always,
    Avoid,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ObjectWrapConfig {
    Preserve,
    Collapse,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EmbeddedLanguageFormattingConfig {
    Auto,
    Off,
}

/// Resolved options from FormatConfig.
#[derive(Debug, Clone)]
pub struct OxfmtOptions {
    pub format_options: FormatOptions,
    pub toml_options: TomlFormatterOptions,
    /// Used when resolving `ExternalFormatterPackageJson` (napi only).
    #[allow(dead_code)]
    pub sort_package_json: bool,
    pub insert_final_newline: bool,
}

impl FormatConfig {
    pub fn into_oxfmt_options(self) -> Result<OxfmtOptions, String> {
        if self.experimental_operator_position.is_some() {
            return Err("Unsupported option: `experimentalOperatorPosition`".to_string());
        }
        if self.experimental_ternaries.is_some() {
            return Err("Unsupported option: `experimentalTernaries`".to_string());
        }

        let mut format_options = FormatOptions {
            // Diverges from oxfmt: preserve explicitly quoted object properties by default.
            quote_properties: QuoteProperties::Preserve,
            ..FormatOptions::default()
        };

        if let Some(use_tabs) = self.use_tabs {
            format_options.indent_style = if use_tabs {
                IndentStyle::Tab
            } else {
                IndentStyle::Space
            };
        }
        if let Some(width) = self.tab_width {
            format_options.indent_width =
                IndentWidth::try_from(width).map_err(|e| format!("Invalid tabWidth: {e}"))?;
        }
        if let Some(ending) = self.end_of_line {
            format_options.line_ending = match ending {
                EndOfLineConfig::Lf => LineEnding::Lf,
                EndOfLineConfig::Crlf => LineEnding::Crlf,
                EndOfLineConfig::Cr => LineEnding::Cr,
            };
        }
        if let Some(width) = self.print_width {
            format_options.line_width =
                LineWidth::try_from(width).map_err(|e| format!("Invalid printWidth: {e}"))?;
        }
        if let Some(single_quote) = self.single_quote {
            format_options.quote_style = if single_quote {
                QuoteStyle::Single
            } else {
                QuoteStyle::Double
            };
        }
        if let Some(jsx_single_quote) = self.jsx_single_quote {
            format_options.jsx_quote_style = if jsx_single_quote {
                QuoteStyle::Single
            } else {
                QuoteStyle::Double
            };
        }
        if let Some(props) = self.quote_props {
            format_options.quote_properties = match props {
                QuotePropsConfig::AsNeeded => QuoteProperties::AsNeeded,
                QuotePropsConfig::Consistent => QuoteProperties::Consistent,
                QuotePropsConfig::Preserve => QuoteProperties::Preserve,
            };
        }
        if let Some(commas) = self.trailing_comma {
            format_options.trailing_commas = match commas {
                TrailingCommaConfig::All => TrailingCommas::All,
                TrailingCommaConfig::Es5 => TrailingCommas::Es5,
                TrailingCommaConfig::None => TrailingCommas::None,
            };
        }
        if let Some(semi) = self.semi {
            format_options.semicolons = if semi {
                Semicolons::Always
            } else {
                Semicolons::AsNeeded
            };
        }
        if let Some(parens) = self.arrow_parens {
            format_options.arrow_parentheses = match parens {
                ArrowParensConfig::Avoid => ArrowParentheses::AsNeeded,
                ArrowParensConfig::Always => ArrowParentheses::Always,
            };
        }
        if let Some(spacing) = self.bracket_spacing {
            format_options.bracket_spacing = BracketSpacing::from(spacing);
        }
        if let Some(same_line) = self.bracket_same_line {
            format_options.bracket_same_line = BracketSameLine::from(same_line);
        }
        if let Some(object_wrap) = self.object_wrap {
            format_options.expand = match object_wrap {
                ObjectWrapConfig::Preserve => Expand::Auto,
                ObjectWrapConfig::Collapse => Expand::Never,
            };
        }
        if let Some(single_attribute_per_line) = self.single_attribute_per_line {
            format_options.attribute_position = if single_attribute_per_line {
                AttributePosition::Multiline
            } else {
                AttributePosition::Auto
            };
        }
        if let Some(embedded_language_formatting) = self.embedded_language_formatting {
            format_options.embedded_language_formatting = match embedded_language_formatting {
                EmbeddedLanguageFormattingConfig::Auto => EmbeddedLanguageFormatting::Auto,
                EmbeddedLanguageFormattingConfig::Off => EmbeddedLanguageFormatting::Off,
            };
        }
        // experimental_sort_imports, experimental_tailwindcss: parsed but not applied in this minimal impl

        let toml_options = build_toml_options(&format_options);
        let sort_package_json = !matches!(
            self.experimental_sort_package_json.as_ref(),
            Some(Value::Bool(false))
        );
        let insert_final_newline = self.insert_final_newline.unwrap_or(true);

        Ok(OxfmtOptions {
            format_options,
            toml_options,
            sort_package_json,
            insert_final_newline,
        })
    }
}

fn build_toml_options(format_options: &FormatOptions) -> TomlFormatterOptions {
    TomlFormatterOptions {
        column_width: format_options.line_width.value() as usize,
        indent_string: if format_options.indent_style.is_tab() {
            "\t".to_string()
        } else {
            " ".repeat(format_options.indent_width.value() as usize)
        },
        array_trailing_comma: !format_options.trailing_commas.is_none(),
        crlf: format_options.line_ending.is_carriage_return_line_feed(),
        trailing_newline: true,
        ..Default::default()
    }
}

/// Populates the raw config JSON with resolved FormatOptions for Prettier / external formatter.
pub fn populate_prettier_config(options: &FormatOptions, config: &mut Value) {
    let Some(obj) = config.as_object_mut() else {
        return;
    };
    obj.insert(
        "printWidth".to_string(),
        Value::from(options.line_width.value()),
    );
    obj.insert(
        "useTabs".to_string(),
        Value::from(match options.indent_style {
            IndentStyle::Tab => true,
            IndentStyle::Space => false,
        }),
    );
    obj.insert(
        "tabWidth".to_string(),
        Value::from(options.indent_width.value()),
    );
    obj.insert(
        "endOfLine".to_string(),
        Value::from(match options.line_ending {
            LineEnding::Lf => "lf",
            LineEnding::Crlf => "crlf",
            LineEnding::Cr => "cr",
        }),
    );
    obj.insert(
        "quoteProps".to_string(),
        Value::from(match options.quote_properties {
            QuoteProperties::AsNeeded => "as-needed",
            QuoteProperties::Consistent => "consistent",
            QuoteProperties::Preserve => "preserve",
        }),
    );
    obj.remove("overrides");
    obj.remove("ignorePatterns");
    obj.remove("insertFinalNewline");
    obj.remove("experimentalSortImports");
    obj.remove("experimentalSortPackageJson");
    if let Some(tailwind) = obj.remove("experimentalTailwindcss")
        && let Some(tw) = tailwind.as_object()
    {
        obj.insert("_tailwindPluginEnabled".to_string(), Value::Bool(true));
        for (src, dst) in [
            ("config", "tailwindConfig"),
            ("stylesheet", "tailwindStylesheet"),
            ("functions", "tailwindFunctions"),
            ("attributes", "tailwindAttributes"),
            ("preserveWhitespace", "tailwindPreserveWhitespace"),
            ("preserveDuplicates", "tailwindPreserveDuplicates"),
        ] {
            if let Some(v) = tw.get(src) {
                obj.insert(dst.to_string(), v.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_quote_props_are_preserved() {
        let options = FormatConfig::default()
            .into_oxfmt_options()
            .expect("default format config should be valid");

        assert_eq!(
            options.format_options.quote_properties,
            QuoteProperties::Preserve
        );
    }

    #[test]
    fn prettier_config_uses_preserve_quote_props_by_default() {
        let options = FormatConfig::default()
            .into_oxfmt_options()
            .expect("default format config should be valid");
        let mut config = Value::Object(serde_json::Map::new());

        populate_prettier_config(&options.format_options, &mut config);

        assert_eq!(config["quoteProps"], Value::String("preserve".to_string()));
    }
}
