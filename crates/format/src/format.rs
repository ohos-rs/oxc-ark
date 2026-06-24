#[cfg(feature = "napi")]
use std::borrow::Cow;
use std::path::Path;

use oxc_allocator::{Allocator, AllocatorPool, Vec as ArenaVec};
use oxc_ast::ast::{
    ArrayExpression, ArrayExpressionElement, Expression, ObjectExpression, ObjectPropertyKind,
    PropertyKey, Statement, StringLiteral,
};
use oxc_diagnostics::OxcDiagnostic;
use oxc_formatter::{JsFormatOptions as FormatOptions, format as format_js};
use oxc_formatter_core::spec::normalize_string;
use oxc_formatter_json::{JsonFormatOptions, JsonVariant, QuoteProps, format as format_json};
use oxc_parser::{ParseOptions, Parser};
use oxc_span::SourceType;
use serde_json::Value;

use super::{FormatFileStrategy, ResolvedOptions};

#[cfg(all(feature = "napi", feature = "sort-package-json"))]
use sort_package_json;

pub enum FormatResult {
    Success { is_changed: bool, code: String },
    Error(Vec<OxcDiagnostic>),
}

pub struct SourceFormatter {
    allocator_pool: AllocatorPool,
    #[cfg(feature = "napi")]
    external_formatter: Option<super::ExternalFormatter>,
}

impl SourceFormatter {
    pub fn new(num_of_threads: usize) -> Self {
        Self {
            allocator_pool: AllocatorPool::new(num_of_threads),
            #[cfg(feature = "napi")]
            external_formatter: None,
        }
    }

    #[cfg(feature = "napi")]
    #[must_use]
    pub fn with_external_formatter(
        mut self,
        external_formatter: Option<super::ExternalFormatter>,
    ) -> Self {
        self.external_formatter = external_formatter;
        self
    }

    /// Format a file based on its entry type and resolved options.
    pub fn format(
        &self,
        entry: &FormatFileStrategy,
        source_text: &str,
        resolved_options: ResolvedOptions,
    ) -> FormatResult {
        let (result, insert_final_newline) = match (entry, resolved_options) {
            (
                FormatFileStrategy::OxcFormatter { path, source_type },
                ResolvedOptions::OxcFormatter {
                    format_options,
                    external_options,
                    insert_final_newline,
                },
            ) => (
                self.format_by_oxc_formatter(
                    source_text,
                    path,
                    *source_type,
                    *format_options,
                    external_options,
                ),
                insert_final_newline,
            ),
            (
                FormatFileStrategy::OxfmtToml { .. },
                ResolvedOptions::OxfmtToml {
                    toml_options,
                    insert_final_newline,
                },
            ) => (
                Ok(Self::format_by_toml(source_text, toml_options)),
                insert_final_newline,
            ),
            (
                FormatFileStrategy::OxfmtJson { json_type: _, .. },
                ResolvedOptions::OxfmtJson {
                    json_options,
                    insert_final_newline,
                },
            ) => (
                self.format_by_json(source_text, json_options),
                insert_final_newline,
            ),
            #[cfg(feature = "napi")]
            (
                FormatFileStrategy::ExternalFormatter { path, parser_name },
                ResolvedOptions::ExternalFormatter {
                    external_options,
                    insert_final_newline,
                },
            ) => (
                self.format_by_external_formatter(source_text, path, parser_name, external_options),
                insert_final_newline,
            ),
            #[cfg(feature = "napi")]
            (
                FormatFileStrategy::ExternalFormatterPackageJson { path, parser_name },
                ResolvedOptions::ExternalFormatterPackageJson {
                    external_options,
                    sort_package_json,
                    insert_final_newline,
                },
            ) => (
                self.format_by_external_formatter_package_json(
                    source_text,
                    path,
                    parser_name,
                    external_options,
                    sort_package_json,
                ),
                insert_final_newline,
            ),
            _ => unreachable!("FormatFileStrategy and ResolvedOptions variant mismatch"),
        };

        match result {
            Ok(mut code) => {
                // NOTE: `insert_final_newline` relies on the fact that:
                // - each formatter already ensures there is trailing newline
                // - each formatter does not have an option to disable trailing newline
                // So we can trim it here without allocating new string.
                if !insert_final_newline {
                    let trimmed_len = code.trim_end().len();
                    code.truncate(trimmed_len);
                }

                FormatResult::Success {
                    is_changed: source_text != code,
                    code,
                }
            }
            Err(err) => FormatResult::Error(vec![err]),
        }
    }

    /// Format JS/TS source code using oxc_formatter.
    fn format_by_oxc_formatter(
        &self,
        source_text: &str,
        path: &Path,
        source_type: SourceType,
        format_options: FormatOptions,
        external_options: Value,
    ) -> Result<String, OxcDiagnostic> {
        let allocator = self.allocator_pool.get();

        #[cfg(feature = "napi")]
        let external_callbacks = {
            let is_embed_off = format_options.embedded_language_formatting.is_off();
            if is_embed_off {
                None
            } else {
                self.external_formatter
                    .as_ref()
                    .map(|ext| ext.to_external_callbacks(path, &format_options, external_options))
            }
        };

        #[cfg(not(feature = "napi"))]
        let external_callbacks = {
            let _ = external_options;
            None
        };

        let formatted = format_js(
            &allocator,
            source_text,
            source_type,
            format_options,
            external_callbacks,
        )?;

        let code = formatted.print().map_err(|err| {
            OxcDiagnostic::error(format!(
                "Failed to print formatted code: {}\n{err}",
                path.display()
            ))
        })?;

        Ok(code.into_code())
    }

    /// Format TOML file using `oxc-toml`.
    fn format_by_toml(source_text: &str, options: oxc_toml::Options) -> String {
        oxc_toml::format(source_text, options)
    }

    /// Format JSON/JSON5/JSONC using `oxc_formatter_json`.
    fn format_by_json(
        &self,
        source_text: &str,
        options: JsonFormatOptions,
    ) -> Result<String, OxcDiagnostic> {
        let preserve_json5_key_quotes = should_preserve_json5_key_quote_style(options);
        let allocator = self.allocator_pool.get();
        let formatted = format_json(&allocator, source_text, options)?;
        let code = formatted.print().map_err(|err| {
            OxcDiagnostic::error(format!("Failed to print formatted JSON: {err}"))
        })?;
        let mut code = code.into_code();

        if preserve_json5_key_quotes {
            restore_json5_double_quoted_keys(source_text, &mut code);
        }

        Ok(code)
    }
}

#[derive(Debug)]
struct Json5KeyQuote {
    value: String,
    quote: u8,
}

#[derive(Debug)]
struct FormattedJson5Key {
    value: String,
    start: usize,
    end: usize,
    raw_inner: String,
    quote: u8,
}

fn should_preserve_json5_key_quote_style(options: JsonFormatOptions) -> bool {
    options.variant == JsonVariant::Json5
        && matches!(options.quote_props, QuoteProps::Preserve)
        && options.single_quote.value()
}

fn restore_json5_double_quoted_keys(source_text: &str, code: &mut String) {
    let Some(source_keys) = collect_source_json5_key_quotes(source_text) else {
        return;
    };
    if !source_keys.iter().any(|key| key.quote == b'"') {
        return;
    }

    let Some(formatted_keys) = collect_formatted_json5_keys(code) else {
        return;
    };

    let mut replacements = Vec::new();
    for (source_key, formatted_key) in source_keys.iter().zip(formatted_keys.iter()) {
        if source_key.quote != b'"'
            || formatted_key.quote != b'\''
            || source_key.value != formatted_key.value
        {
            continue;
        }

        let normalized = normalize_string(&formatted_key.raw_inner, b'"', true);
        replacements.push((
            formatted_key.start,
            formatted_key.end,
            format!("\"{normalized}\""),
        ));
    }

    for (start, end, replacement) in replacements.into_iter().rev() {
        if start <= end && end <= code.len() {
            code.replace_range(start..end, &replacement);
        }
    }
}

fn collect_source_json5_key_quotes(source_text: &str) -> Option<Vec<Json5KeyQuote>> {
    let allocator = Allocator::default();
    let expression = parse_json_expression(&allocator, source_text)?;
    let mut keys = Vec::new();
    collect_source_key_quotes_from_expression(expression, &mut keys);
    Some(keys)
}

fn collect_formatted_json5_keys(code: &str) -> Option<Vec<FormattedJson5Key>> {
    let allocator = Allocator::default();
    let expression = parse_json_expression(&allocator, code)?;
    let mut keys = Vec::new();
    collect_formatted_keys_from_expression(expression, &mut keys);
    Some(keys)
}

fn parse_json_expression<'a>(
    allocator: &'a Allocator,
    source_text: &str,
) -> Option<&'a Expression<'a>> {
    let wrapped_source = allocator.alloc_concat_strs_array(["(", source_text, "\n)"]);
    let options = ParseOptions {
        preserve_parens: false,
        ..ParseOptions::default()
    };
    let ret = Parser::new(allocator, wrapped_source, SourceType::default())
        .with_options(options)
        .parse();
    if ret.panicked || !ret.diagnostics.is_empty() {
        return None;
    }

    let mut program = ret.program;
    let body = std::mem::replace(&mut program.body, ArenaVec::new_in(allocator)).into_arena_slice();
    let Statement::ExpressionStatement(statement) = body.first()? else {
        return None;
    };
    Some(&statement.expression)
}

fn collect_source_key_quotes_from_expression(
    expression: &Expression<'_>,
    keys: &mut Vec<Json5KeyQuote>,
) {
    match expression {
        Expression::ObjectExpression(object) => collect_source_key_quotes_from_object(object, keys),
        Expression::ArrayExpression(array) => collect_source_key_quotes_from_array(array, keys),
        Expression::ParenthesizedExpression(expression) => {
            collect_source_key_quotes_from_expression(&expression.expression, keys);
        }
        _ => {}
    }
}

fn collect_source_key_quotes_from_array(
    array: &ArrayExpression<'_>,
    keys: &mut Vec<Json5KeyQuote>,
) {
    for element in &array.elements {
        match element {
            ArrayExpressionElement::ObjectExpression(object) => {
                collect_source_key_quotes_from_object(object, keys);
            }
            ArrayExpressionElement::ArrayExpression(array) => {
                collect_source_key_quotes_from_array(array, keys);
            }
            ArrayExpressionElement::ParenthesizedExpression(expression) => {
                collect_source_key_quotes_from_expression(&expression.expression, keys);
            }
            _ => {}
        }
    }
}

fn collect_source_key_quotes_from_object(
    object: &ObjectExpression<'_>,
    keys: &mut Vec<Json5KeyQuote>,
) {
    for property in &object.properties {
        let ObjectPropertyKind::ObjectProperty(property) = property else {
            continue;
        };

        if let PropertyKey::StringLiteral(lit) = &property.key
            && let Some(quote) = string_literal_quote(lit)
        {
            keys.push(Json5KeyQuote {
                value: lit.value.to_string(),
                quote,
            });
        }
        collect_source_key_quotes_from_expression(&property.value, keys);
    }
}

fn collect_formatted_keys_from_expression(
    expression: &Expression<'_>,
    keys: &mut Vec<FormattedJson5Key>,
) {
    match expression {
        Expression::ObjectExpression(object) => collect_formatted_keys_from_object(object, keys),
        Expression::ArrayExpression(array) => collect_formatted_keys_from_array(array, keys),
        Expression::ParenthesizedExpression(expression) => {
            collect_formatted_keys_from_expression(&expression.expression, keys);
        }
        _ => {}
    }
}

fn collect_formatted_keys_from_array(
    array: &ArrayExpression<'_>,
    keys: &mut Vec<FormattedJson5Key>,
) {
    for element in &array.elements {
        match element {
            ArrayExpressionElement::ObjectExpression(object) => {
                collect_formatted_keys_from_object(object, keys);
            }
            ArrayExpressionElement::ArrayExpression(array) => {
                collect_formatted_keys_from_array(array, keys);
            }
            ArrayExpressionElement::ParenthesizedExpression(expression) => {
                collect_formatted_keys_from_expression(&expression.expression, keys);
            }
            _ => {}
        }
    }
}

fn collect_formatted_keys_from_object(
    object: &ObjectExpression<'_>,
    keys: &mut Vec<FormattedJson5Key>,
) {
    for property in &object.properties {
        let ObjectPropertyKind::ObjectProperty(property) = property else {
            continue;
        };

        if let PropertyKey::StringLiteral(lit) = &property.key
            && let Some((quote, raw_inner)) = string_literal_quote_and_inner(lit)
        {
            keys.push(FormattedJson5Key {
                value: lit.value.to_string(),
                start: lit.span.start.saturating_sub(1) as usize,
                end: lit.span.end.saturating_sub(1) as usize,
                raw_inner,
                quote,
            });
        }
        collect_formatted_keys_from_expression(&property.value, keys);
    }
}

fn string_literal_quote(lit: &StringLiteral<'_>) -> Option<u8> {
    string_literal_quote_and_inner(lit).map(|(quote, _)| quote)
}

fn string_literal_quote_and_inner(lit: &StringLiteral<'_>) -> Option<(u8, String)> {
    let raw = lit.raw.as_ref()?.as_str();
    let bytes = raw.as_bytes();
    let quote = *bytes.first()?;
    if !matches!(quote, b'"' | b'\'') || bytes.last().copied() != Some(quote) {
        return None;
    }
    let inner = raw.get(1..raw.len().checked_sub(1)?)?.to_string();
    Some((quote, inner))
}

impl SourceFormatter {
    /// Format non-JS/TS file using external formatter (Prettier).
    #[cfg(feature = "napi")]
    #[expect(clippy::needless_pass_by_value)]
    fn format_by_external_formatter(
        &self,
        source_text: &str,
        path: &Path,
        parser_name: &str,
        external_options: Value,
    ) -> Result<String, OxcDiagnostic> {
        let external_formatter = self
            .external_formatter
            .as_ref()
            .expect("`external_formatter` must exist when `napi` feature is enabled");

        // NOTE: To call Prettier, we need to either:
        // - let Prettier infer the parser from `filepath`
        // - or specify the `parser`
        //
        // We are specifying the `parser` for perf, so `filepath` is not actually necessary,
        // but since some plugins might depend on `filepath`, we pass the actual file name as well.
        let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");

        external_formatter
            .format_file(&external_options, parser_name, file_name, source_text)
            .map_err(|err| {
                OxcDiagnostic::error(format!(
                    "Failed to format file with external formatter: {}\n{err}",
                    path.display()
                ))
            })
    }

    /// Format `package.json`: optionally sort then format by external formatter.
    #[cfg(feature = "napi")]
    fn format_by_external_formatter_package_json(
        &self,
        source_text: &str,
        path: &Path,
        parser_name: &str,
        external_options: Value,
        sort_package_json: bool,
    ) -> Result<String, OxcDiagnostic> {
        let source_text: Cow<'_, str> = if sort_package_json {
            #[cfg(feature = "sort-package-json")]
            {
                Cow::Owned(
                    sort_package_json::sort_package_json(source_text).map_err(|err| {
                        OxcDiagnostic::error(format!(
                            "Failed to sort package.json: {}\n{err}",
                            path.display()
                        ))
                    })?,
                )
            }
            #[cfg(not(feature = "sort-package-json"))]
            {
                return Err(OxcDiagnostic::error(
                    "sort-package-json feature is required to sort package.json files".to_string(),
                ));
            }
        } else {
            Cow::Borrowed(source_text)
        };

        self.format_by_external_formatter(&source_text, path, parser_name, external_options)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use oxc_formatter_core::{IndentStyle, LineEnding};
    use oxc_formatter_json::{JsonFormatOptions, JsonVariant, QuoteProps, SingleQuote};

    fn format_json_source(source: &str, options: JsonFormatOptions) -> String {
        SourceFormatter::new(1)
            .format_by_json(source, options)
            .expect("JSON formatting should succeed")
    }

    #[test]
    fn formats_json_with_upstream_json_variant() {
        let formatted = format_json_source(
            r#"{"name":"test","version":"1.0.0"}"#,
            JsonFormatOptions {
                variant: JsonVariant::Json,
                ..JsonFormatOptions::default()
            },
        );

        assert_eq!(
            formatted,
            "{ \"name\": \"test\", \"version\": \"1.0.0\" }\n"
        );
    }

    #[test]
    fn formats_jsonc_with_upstream_comment_preservation() {
        let formatted = format_json_source(
            r#"{
  // This is a comment
  "name": "test"
}"#,
            JsonFormatOptions {
                variant: JsonVariant::Jsonc,
                ..JsonFormatOptions::default()
            },
        );

        assert!(
            formatted.contains("// This is a comment"),
            "JSONC comments should follow upstream behavior and be preserved"
        );
        assert!(formatted.contains("\"name\": \"test\""));
    }

    #[test]
    fn formats_json5_with_upstream_json5_variant() {
        let formatted = format_json_source(
            r#"{
  // This is a JSON5 file
  name: 'test',
  version: '1.0.0'
}"#,
            JsonFormatOptions {
                variant: JsonVariant::Json5,
                single_quote: SingleQuote::from(true),
                ..JsonFormatOptions::default()
            },
        );

        assert!(formatted.contains("// This is a JSON5 file"));
        assert!(formatted.contains("name: 'test'"));
        assert!(formatted.contains("version: '1.0.0'"));
    }

    #[test]
    fn formats_json5_with_tabs_and_crlf() {
        let formatted = format_json_source(
            r#"{
  name: "test",
  nested: {
    value: true
  }
}"#,
            JsonFormatOptions {
                variant: JsonVariant::Json5,
                indent_style: IndentStyle::Tab,
                line_ending: LineEnding::Crlf,
                ..JsonFormatOptions::default()
            },
        );

        assert!(formatted.contains("\r\n\tname"));
        assert!(formatted.ends_with("\r\n"));
    }

    #[test]
    fn reports_invalid_json5_syntax() {
        let result = SourceFormatter::new(1).format_by_json(
            r#"{
  name: 'test',
  invalid: [unclosed array
}"#,
            JsonFormatOptions {
                variant: JsonVariant::Json5,
                ..JsonFormatOptions::default()
            },
        );

        assert!(result.is_err(), "Invalid JSON5 should return an error");
    }

    #[test]
    fn json5_preserve_quote_props_keeps_mixed_keys() {
        let formatted = format_json_source(
            r#"{
  "quoted": 'value',
  plain: 'other'
}"#,
            JsonFormatOptions {
                variant: JsonVariant::Json5,
                single_quote: SingleQuote::from(true),
                quote_props: QuoteProps::Preserve,
                ..JsonFormatOptions::default()
            },
        );

        assert!(formatted.contains("\"quoted\": 'value'"));
        assert!(formatted.contains("plain: 'other'"));
    }
}
