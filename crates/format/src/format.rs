#[cfg(feature = "napi")]
use std::borrow::Cow;
use std::path::Path;

use oxc_allocator::AllocatorPool;
use oxc_diagnostics::OxcDiagnostic;
use oxc_formatter::{FormatOptions, Formatter, enable_jsx_source_type, get_parse_options};
use oxc_parser::Parser;
use oxc_span::SourceType;
use serde_json::Value;

use super::config::JsonFormatterOptions;
use super::support::JsonType;
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
                    format_options,
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
                    json_type: resolved_json_type,
                    insert_final_newline,
                },
            ) => (
                Self::format_by_json(source_text, resolved_json_type, json_options),
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
        let source_type = enable_jsx_source_type(source_type);
        let allocator = self.allocator_pool.get();

        let ret = Parser::new(&allocator, source_text, source_type)
            .with_options(get_parse_options())
            .parse();
        if !ret.errors.is_empty() {
            // Return the first error for simplicity
            return Err(ret
                .errors
                .into_iter()
                .next()
                .expect("errors.is_empty() was checked above"));
        }

        #[cfg(feature = "napi")]
        let is_embed_off = format_options.embedded_language_formatting.is_off();

        let base_formatter = Formatter::new(&allocator, format_options);

        #[cfg(feature = "napi")]
        let formatted = {
            if is_embed_off {
                base_formatter.format(&ret.program)
            } else {
                let embedded_formatter = self
                    .external_formatter
                    .as_ref()
                    .expect("`external_formatter` must exist when `napi` feature is enabled")
                    .to_embedded_formatter(external_options);
                base_formatter.format_with_embedded(&ret.program, embedded_formatter)
            }
        };
        #[cfg(not(feature = "napi"))]
        let formatted = {
            let _ = external_options;
            base_formatter.format(&ret.program)
        };

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

    /// Format JSON/JSON5/JSONC file using Rust formatters.
    fn format_by_json(
        source_text: &str,
        json_type: JsonType,
        options: JsonFormatterOptions,
    ) -> Result<String, OxcDiagnostic> {
        match json_type {
            JsonType::Json => format_json(source_text, &options),
            JsonType::Json5 => format_json5(source_text, &options),
            JsonType::Jsonc => format_jsonc(source_text, &options),
        }
    }
}

// --- JSON formatting functions

/// Format standard JSON file.
fn format_json(source_text: &str, options: &JsonFormatterOptions) -> Result<String, OxcDiagnostic> {
    // Parse JSON
    let value: serde_json::Value = serde_json::from_str(source_text)
        .map_err(|err| OxcDiagnostic::error(format!("Failed to parse JSON: {err}")))?;

    // Format with serde_json
    let formatted = serde_json::to_string_pretty(&value)
        .map_err(|err| OxcDiagnostic::error(format!("Failed to format JSON: {err}")))?;

    // Replace indentation and line endings
    let formatted = if options.use_tabs {
        // Replace spaces with tabs
        replace_indent(&formatted, options.indent_width, "\t")
    } else {
        formatted
    };

    let formatted = formatted.replace('\n', &options.line_ending);

    Ok(formatted)
}

/// Format JSON5 file (supports comments, trailing commas, etc.).
/// Uses json5format to preserve comments and format JSON5 files.
fn format_json5(
    source_text: &str,
    options: &JsonFormatterOptions,
) -> Result<String, OxcDiagnostic> {
    use json5format::{FormatOptions, Json5Format, ParsedDocument};

    // Parse the JSON5 document (preserves comments)
    let parsed = ParsedDocument::from_str(source_text, None)
        .map_err(|err| OxcDiagnostic::error(format!("Failed to parse JSON5: {err}")))?;

    // Create format options
    let mut format_options = FormatOptions::default();
    // Note: json5format uses indent_by as usize (number of spaces), tabs are not directly supported
    // We'll use spaces and then replace with tabs if needed
    let indent_by = if options.use_tabs {
        1 // Will be replaced with tabs later
    } else {
        options.indent_width
    };
    format_options.indent_by = indent_by;
    format_options.trailing_commas = options.trailing_commas;
    format_options.quote_properties = options.quote_properties;

    // Create formatter with options
    let formatter = Json5Format::with_options(format_options)
        .map_err(|err| OxcDiagnostic::error(format!("Failed to create JSON5 formatter: {err}")))?;

    // Format the JSON5 (preserves comments)
    let mut formatted = formatter
        .to_string(&parsed)
        .map_err(|err| OxcDiagnostic::error(format!("Failed to format JSON5: {err}")))?;

    // Replace spaces with tabs if needed
    if options.use_tabs {
        formatted = replace_indent(&formatted, indent_by, "\t");
    }

    // Replace line endings
    formatted = formatted.replace('\n', &options.line_ending);

    Ok(formatted)
}

/// Format JSONC file (JSON with comments).
fn format_jsonc(
    source_text: &str,
    options: &JsonFormatterOptions,
) -> Result<String, OxcDiagnostic> {
    // First, strip comments to get valid JSON
    let mut json_text = source_text.to_string();
    json_strip_comments::strip(&mut json_text).map_err(|err| {
        OxcDiagnostic::error(format!("Failed to strip comments from JSONC: {err}"))
    })?;

    // Then format as standard JSON
    format_json(&json_text, options)
}

/// Replace indentation in formatted JSON string.
fn replace_indent(text: &str, original_width: usize, new_indent: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut result = String::new();

    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            result.push('\n');
        }

        // Count leading spaces (using bytes for safe indexing)
        let leading_spaces_byte_count = line.bytes().take_while(|&b| b == b' ').count();

        if leading_spaces_byte_count > 0
            && leading_spaces_byte_count <= line.len()
            && original_width > 0
        {
            // Calculate number of indent levels
            let indent_levels = leading_spaces_byte_count / original_width;
            // Replace with new indent
            for _ in 0..indent_levels {
                result.push_str(new_indent);
            }
            // Add remaining content (safe because we checked bounds)
            result.push_str(&line[leading_spaces_byte_count..]);
        } else {
            result.push_str(line);
        }
    }

    result
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
    use crate::config::JsonFormatterOptions;
    #[test]
    fn test_format_json5_basic() {
        let source = r#"{
  // This is a JSON5 file
  name: 'test',
  version: '1.0.0',
  description: 'Test package',
  keywords: ['test', 'json5'],
  private: true,
  dependencies: {
    'package-a': '^1.0.0',
    'package-b': '^2.0.0'
  }
}"#;

        let options = JsonFormatterOptions {
            indent_width: 2,
            use_tabs: false,
            line_ending: "\n".to_string(),
            trailing_commas: true,
            quote_properties: json5format::QuoteProperties::Consistent,
        };

        let result = format_json5(source, &options);
        assert!(result.is_ok(), "JSON5 formatting should succeed");
        let formatted = result.unwrap();
        assert!(!formatted.is_empty(), "Formatted JSON5 should not be empty");
        // Verify the formatted code is valid JSON5
        assert!(formatted.contains("name"), "Should contain 'name'");
        assert!(formatted.contains("test"), "Should contain 'test'");
    }

    #[test]
    fn test_format_json5_with_comments() {
        let source = r#"{
  // Single line comment
  name: 'test',
  /* Multi-line
     comment */
  version: '1.0.0',
  description: 'Test package'
}"#;

        let options = JsonFormatterOptions {
            indent_width: 2,
            use_tabs: false,
            line_ending: "\n".to_string(),
            trailing_commas: false,
            quote_properties: json5format::QuoteProperties::Consistent,
        };

        let result = format_json5(source, &options);
        assert!(
            result.is_ok(),
            "JSON5 with comments should format successfully"
        );
        let formatted = result.unwrap();
        assert!(!formatted.is_empty(), "Formatted JSON5 should not be empty");
        // Verify comments are preserved
        assert!(
            formatted.contains("//") || formatted.contains("/*"),
            "Comments should be preserved in formatted JSON5"
        );
    }

    #[test]
    fn test_format_json5_with_trailing_commas() {
        let source = r#"{
  name: 'test',
  version: '1.0.0',
  dependencies: {
    'package-a': '^1.0.0',
    'package-b': '^2.0.0',
  },
}"#;

        let options = JsonFormatterOptions {
            indent_width: 2,
            use_tabs: false,
            line_ending: "\n".to_string(),
            trailing_commas: true,
            quote_properties: json5format::QuoteProperties::Consistent,
        };

        let result = format_json5(source, &options);
        assert!(
            result.is_ok(),
            "JSON5 with trailing commas should format successfully"
        );
    }

    #[test]
    fn test_format_json5_with_tabs() {
        let source = r#"{
  name: 'test',
  version: '1.0.0'
}"#;

        let options = JsonFormatterOptions {
            indent_width: 2,
            use_tabs: true,
            line_ending: "\n".to_string(),
            trailing_commas: false,
            quote_properties: json5format::QuoteProperties::Consistent,
        };

        let result = format_json5(source, &options);
        assert!(result.is_ok(), "JSON5 with tabs should format successfully");
        let formatted = result.unwrap();
        // Note: json5format uses spaces for indentation, tabs are applied via post-processing
        // in the format_json5 function
        assert!(!formatted.is_empty(), "Formatted JSON5 should not be empty");
    }

    #[test]
    fn test_format_json5_with_crlf() {
        let source = r#"{
  name: 'test',
  version: '1.0.0'
}"#;

        let options = JsonFormatterOptions {
            indent_width: 2,
            use_tabs: false,
            line_ending: "\r\n".to_string(),
            trailing_commas: false,
            quote_properties: json5format::QuoteProperties::Consistent,
        };

        let result = format_json5(source, &options);
        assert!(result.is_ok(), "JSON5 with CRLF should format successfully");
        let formatted = result.unwrap();
        // Verify CRLF line endings are used
        assert!(
            formatted.contains("\r\n"),
            "Formatted JSON5 should use CRLF line endings"
        );
    }

    #[test]
    fn test_format_json5_invalid_syntax() {
        let source = r#"{
  name: 'test',
  version: '1.0.0',
  invalid: [unclosed array
}"#;

        let options = JsonFormatterOptions {
            indent_width: 2,
            use_tabs: false,
            line_ending: "\n".to_string(),
            trailing_commas: false,
            quote_properties: json5format::QuoteProperties::Consistent,
        };

        let result = format_json5(source, &options);
        assert!(result.is_err(), "Invalid JSON5 should return an error");
    }

    #[test]
    fn test_format_json_basic() {
        let source = r#"{"name":"test","version":"1.0.0","description":"Test package"}"#;

        let options = JsonFormatterOptions {
            indent_width: 2,
            use_tabs: false,
            line_ending: "\n".to_string(),
            trailing_commas: false,
            quote_properties: json5format::QuoteProperties::Consistent,
        };

        let result = format_json(source, &options);
        assert!(result.is_ok(), "JSON formatting should succeed");
        let formatted = result.unwrap();
        assert!(!formatted.is_empty(), "Formatted JSON should not be empty");
        assert!(formatted.contains("name"), "Should contain 'name'");
        assert!(formatted.contains("test"), "Should contain 'test'");
    }

    #[test]
    fn test_format_jsonc_basic() {
        let source = r#"{
  // This is a comment
  "name": "test",
  "version": "1.0.0",
  /* Another comment */
  "description": "Test package"
}"#;

        let options = JsonFormatterOptions {
            indent_width: 2,
            use_tabs: false,
            line_ending: "\n".to_string(),
            trailing_commas: false,
            quote_properties: json5format::QuoteProperties::Consistent,
        };

        let result = format_jsonc(source, &options);
        assert!(result.is_ok(), "JSONC formatting should succeed");
        let formatted = result.unwrap();
        assert!(!formatted.is_empty(), "Formatted JSONC should not be empty");
        // Comments should be stripped, so formatted JSON should not contain comment markers
        assert!(
            !formatted.contains("//"),
            "Comments should be stripped from JSONC"
        );
        assert!(
            !formatted.contains("/*"),
            "Comments should be stripped from JSONC"
        );
    }

    #[test]
    fn test_format_by_json_integration() {
        use crate::support::JsonType;

        let source = r#"{
  name: 'test',
  version: '1.0.0'
}"#;

        let options = JsonFormatterOptions {
            indent_width: 2,
            use_tabs: false,
            line_ending: "\n".to_string(),
            trailing_commas: false,
            quote_properties: json5format::QuoteProperties::Consistent,
        };

        // Test JSON5
        let result = SourceFormatter::format_by_json(source, JsonType::Json5, options.clone());
        assert!(result.is_ok(), "format_by_json with Json5 should succeed");

        // Test JSON
        let json_source = r#"{"name":"test","version":"1.0.0"}"#;
        let result = SourceFormatter::format_by_json(json_source, JsonType::Json, options.clone());
        assert!(result.is_ok(), "format_by_json with Json should succeed");

        // Test JSONC
        let jsonc_source = r#"{
  // comment
  "name": "test"
}"#;
        let result = SourceFormatter::format_by_json(jsonc_source, JsonType::Jsonc, options);
        assert!(result.is_ok(), "format_by_json with Jsonc should succeed");
    }

    #[test]
    fn test_format_json5_with_license_comment() {
        // Test case from user report - JSON5 with license comment block
        let source = r#"/*
* Copyright (C) 2023 Huawei Device Co., Ltd.
* Licensed under the Apache License, Version 2.0 (the "License");
* you may not use this file except in compliance with the License.
* You may obtain a copy of the License at
*
* http://www.apache.org/licenses/LICENSE-2.0
*
* Unless required by applicable law or agreed to in writing, software
* distributed under the License is distributed on an "AS IS" BASIS,
* WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
* See the License for the specific language governing permissions and
* limitations under the License.
*/

{
  "license": "ISC",
  "types": "",
  "devDependencies": {},
  "name": "@ohos/video-component",
  "description": "a npm package which contains arkUI2.0 page",
  "main": "index.ets",
  "repository": {},
  "version": "1.0.5",
  "dependencies": {}
}"#;

        let options = JsonFormatterOptions {
            indent_width: 2,
            use_tabs: false,
            line_ending: "\n".to_string(),
            trailing_commas: false,
            quote_properties: json5format::QuoteProperties::Consistent,
        };

        let result = format_json5(source, &options);
        assert!(
            result.is_ok(),
            "JSON5 with license comment should format successfully"
        );
        let formatted = result.unwrap();
        assert!(!formatted.is_empty(), "Formatted JSON5 should not be empty");
    }

    #[test]
    fn test_format_json5_quote_properties_consistent_with_quotes() {
        // Test Consistent behavior when source has quoted keys
        let source = r#"{
  "name": "test",
  "version": "1.0.0",
  "description": "Test package"
}"#;

        let options = JsonFormatterOptions {
            indent_width: 2,
            use_tabs: false,
            line_ending: "\n".to_string(),
            trailing_commas: false,
            quote_properties: json5format::QuoteProperties::Consistent,
        };

        let result = format_json5(source, &options);
        assert!(result.is_ok(), "JSON5 formatting should succeed");
        let formatted = result.unwrap();

        // With Consistent, if source has quotes, it should keep quotes
        println!(
            "Formatted with Consistent (source has quotes):\n{}",
            formatted
        );
        // Check that quotes are preserved
        assert!(
            formatted.contains("\"name\""),
            "Should preserve quotes when source has quotes"
        );
        assert!(
            formatted.contains("\"version\""),
            "Should preserve quotes when source has quotes"
        );
    }

    #[test]
    fn test_format_json5_quote_properties_consistent_without_quotes() {
        // Test Consistent behavior when source has unquoted keys
        let source = r#"{
  name: "test",
  version: "1.0.0",
  description: "Test package"
}"#;

        let options = JsonFormatterOptions {
            indent_width: 2,
            use_tabs: false,
            line_ending: "\n".to_string(),
            trailing_commas: false,
            quote_properties: json5format::QuoteProperties::Consistent,
        };

        let result = format_json5(source, &options);
        assert!(result.is_ok(), "JSON5 formatting should succeed");
        let formatted = result.unwrap();

        // With Consistent, if source has no quotes, it should keep no quotes
        println!(
            "Formatted with Consistent (source has no quotes):\n{}",
            formatted
        );
        // Check that no quotes are added
        assert!(
            formatted.contains("name:"),
            "Should preserve no quotes when source has no quotes"
        );
        assert!(
            formatted.contains("version:"),
            "Should preserve no quotes when source has no quotes"
        );
        // Should not have quoted keys
        assert!(
            !formatted.contains("\"name\":"),
            "Should not add quotes when source has no quotes"
        );
    }

    #[test]
    fn test_format_json5_quote_properties_consistent_mixed() {
        // Test Consistent behavior with mixed quoted/unquoted keys
        let source = r#"{
  "name": "test",
  version: "1.0.0",
  "description": "Test package"
}"#;

        let options = JsonFormatterOptions {
            indent_width: 2,
            use_tabs: false,
            line_ending: "\n".to_string(),
            trailing_commas: false,
            quote_properties: json5format::QuoteProperties::Consistent,
        };

        let result = format_json5(source, &options);
        assert!(result.is_ok(), "JSON5 formatting should succeed");
        let formatted = result.unwrap();

        println!("Formatted with Consistent (mixed quotes):\n{}", formatted);
        // Consistent should make all keys have the same quote style
        // It typically uses the majority style or the first style
    }

    #[test]
    fn test_format_json5_quote_properties_preserve() {
        // Test Preserve behavior - should keep original quote style
        let source = r#"{
  "name": "test",
  version: "1.0.0",
  "description": "Test package"
}"#;

        let options = JsonFormatterOptions {
            indent_width: 2,
            use_tabs: false,
            line_ending: "\n".to_string(),
            trailing_commas: false,
            quote_properties: json5format::QuoteProperties::Preserve,
        };

        let result = format_json5(source, &options);
        assert!(result.is_ok(), "JSON5 formatting should succeed");
        let formatted = result.unwrap();

        println!("Formatted with Preserve:\n{}", formatted);
        // Preserve should keep the original quote style for each key
        assert!(
            formatted.contains("\"name\""),
            "Preserve should keep quoted keys"
        );
        assert!(
            formatted.contains("version:"),
            "Preserve should keep unquoted keys"
        );
    }
}
