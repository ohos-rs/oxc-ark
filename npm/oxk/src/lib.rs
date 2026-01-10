#![deny(clippy::all)]

use napi_derive::napi;
use serde_json::Value;
use std::path::PathBuf;

use format::{
  should_ignore_file, ConfigResolver, ExternalFormatter, FormatFileStrategy,
  FormatResult as CoreFormatResult, JsFormatEmbeddedCb, JsFormatFileCb, JsInitExternalFormatterCb,
  ResolvedOptions, SourceFormatter,
};

#[napi(object)]
pub struct FormatResult {
  /// The formatted code.
  pub code: String,
  /// Parse and format errors.
  pub errors: Vec<String>,
}

/// Format a file with the given options.
///
/// This function supports multiple file types:
/// - JavaScript/TypeScript files (via oxc_formatter)
/// - TOML files (via oxc_toml)
/// - Other files (via external formatter callbacks when napi feature is enabled)
#[napi]
pub async fn format(
  filename: String,
  source_text: String,
  options: Option<Value>,
  #[napi(ts_arg_type = "(numThreads: number) => Promise<string[]>")]
  init_external_formatter_cb: Option<JsInitExternalFormatterCb>,
  #[napi(
    ts_arg_type = "(options: Record<string, any>, tagName: string, code: string) => Promise<string>"
  )]
  format_embedded_cb: Option<JsFormatEmbeddedCb>,
  #[napi(
    ts_arg_type = "(options: Record<string, any>, parserName: string, fileName: string, code: string) => Promise<string>"
  )]
  format_file_cb: Option<JsFormatFileCb>,
) -> FormatResult {
  let num_of_threads = 1;

  // Create external formatter if callbacks are provided
  let external_formatter = if let (Some(init_cb), Some(embedded_cb), Some(file_cb)) = (
    init_external_formatter_cb,
    format_embedded_cb,
    format_file_cb,
  ) {
    Some(ExternalFormatter::new(init_cb, embedded_cb, file_cb))
  } else {
    None
  };

  // Create resolver from options and resolve format options
  let config_value = options.unwrap_or_else(|| Value::Object(serde_json::Map::new()));
  let mut config_resolver = ConfigResolver::from_value(config_value);
  match config_resolver.build_and_validate() {
    Ok(_) => {}
    Err(err) => {
      return FormatResult {
        code: source_text,
        errors: vec![format!("Failed to parse configuration: {err}")],
      };
    }
  }

  // Initialize external formatter if provided
  if let Some(ref ext_fmt) = external_formatter {
    #[cfg(not(target_family = "wasm"))]
    let init_result = tokio::task::block_in_place(|| ext_fmt.init(num_of_threads));
    #[cfg(target_family = "wasm")]
    {
      // In wasm, we're already in an async context, so we can't use block_on.
      // The ext_fmt.init() uses block_on internally, which will fail in wasm with
      // "Cannot start a runtime from within a runtime" error.
      // The solution is to skip initialization in wasm, as it's not critical
      // for basic formatting operations. The formatter will still work without it.
      // The init() call is mainly used to get the list of supported languages,
      // which is not required for the formatter to work.
      // TODO: Make init async-aware in wasm to properly support external formatters.
    }

    #[cfg(not(target_family = "wasm"))]
    match init_result {
      Ok(_) => {}
      Err(err) => {
        return FormatResult {
          code: source_text,
          errors: vec![format!("Failed to setup external formatter: {err}")],
        };
      }
    }
  }

  // Skip ignored files silently (e.g., lock files, ignored JSON files)
  if should_ignore_file(PathBuf::from(&filename).as_path()) {
    return FormatResult {
      code: source_text,
      errors: vec![],
    };
  }

  // Determine format strategy from file path
  let Ok(strategy) = FormatFileStrategy::try_from(PathBuf::from(&filename)) else {
    return FormatResult {
      code: source_text,
      errors: vec![format!("Unsupported file type: {filename}")],
    };
  };

  // Check if external formatter is needed but not provided
  // For non-JS/TS/TOML files, external formatter is required
  match &strategy {
    FormatFileStrategy::OxcFormatter { .. } | FormatFileStrategy::OxfmtToml { .. } => {
      // These can be formatted without external formatter
    }
    _ => {
      if external_formatter.is_none() {
        return FormatResult {
          code: source_text,
          errors: vec![format!(
            "External formatter is required for file type: {filename}"
          )],
        };
      }
    }
  }

  let mut resolved_options = config_resolver.resolve(&strategy);

  // Fix quote_properties: Oxfmtrc's deserialization may not properly handle quoteProperties,
  // so we manually override it to Always for JSON/JSON5/JSONC files
  if let ResolvedOptions::OxfmtJson { json_options, .. } = &mut resolved_options {
    json_options.quote_properties = json5format::QuoteProperties::Always;
  }

  // Create formatter
  let formatter = SourceFormatter::new(num_of_threads).with_external_formatter(external_formatter);

  // Format the file
  #[cfg(not(target_family = "wasm"))]
  let format_result =
    tokio::task::block_in_place(|| formatter.format(&strategy, &source_text, resolved_options));
  #[cfg(target_family = "wasm")]
  let format_result = formatter.format(&strategy, &source_text, resolved_options);

  match format_result {
    CoreFormatResult::Success { code, .. } => FormatResult {
      code,
      errors: vec![],
    },
    CoreFormatResult::Error(diagnostics) => {
      let errors: Vec<String> = diagnostics.iter().map(|d| format!("{}", d)).collect();
      FormatResult {
        code: source_text,
        errors,
      }
    }
  }
}
