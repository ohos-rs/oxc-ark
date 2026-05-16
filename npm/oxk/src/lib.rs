#![deny(clippy::all)]

mod parse;

use napi_derive::napi;
use serde_json::Value;
#[cfg(not(target_family = "wasm"))]
use std::ffi::OsString;
use std::path::PathBuf;

use format::{
  should_ignore_file, ConfigResolver, ExternalFormatter, FormatFileStrategy,
  FormatResult as CoreFormatResult, JsFormatEmbeddedCb, JsFormatFileCb, JsInitExternalFormatterCb,
  SourceFormatter,
};
#[cfg(not(target_family = "wasm"))]
use lint::{
  JsCreateWorkspaceCb, JsDestroyWorkspaceCb, JsLintFileCb, JsLoadJsConfigsCb, JsLoadPluginCb,
  JsSetupRuleConfigsCb,
};

#[napi(object)]
pub struct FormatResult {
  /// The formatted code.
  pub code: String,
  /// Parse and format errors.
  pub errors: Vec<String>,
}

#[cfg(not(target_family = "wasm"))]
fn package_version() -> String {
  serde_json::from_str::<Value>(include_str!("../package.json"))
    .ok()
    .and_then(|package| {
      package
        .get("version")
        .and_then(Value::as_str)
        .map(str::to_owned)
    })
    .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string())
}

/// Run the oxfmt-compatible formatter language server.
#[cfg(not(target_family = "wasm"))]
#[napi]
pub async fn format_lsp() -> bool {
  format::run_lsp("oxfmt".to_string(), package_version()).await;
  true
}

/// Run the oxfmt-compatible formatter language server.
#[cfg(target_family = "wasm")]
#[napi]
pub async fn format_lsp() -> napi::Result<bool> {
  Err(napi::Error::from_reason(
    "oxk format --lsp is not supported in WASI builds. Use the native npm package or the cargo CLI.",
  ))
}

/// Run the oxlint-compatible linter.
#[cfg(not(target_family = "wasm"))]
#[napi]
pub async fn lint(args: Vec<String>) -> bool {
  let args = args.into_iter().map(OsString::from).collect();
  lint::lint_args(args)
}

/// Run the oxlint-compatible linter synchronously.
#[cfg(not(target_family = "wasm"))]
#[napi]
pub fn lint_sync(args: Vec<String>) -> bool {
  let args = args.into_iter().map(OsString::from).collect();
  lint::lint_args(args)
}

/// Run the oxlint-compatible linter.
#[cfg(target_family = "wasm")]
#[napi]
pub async fn lint(_args: Vec<String>) -> napi::Result<bool> {
  Err(napi::Error::from_reason(
    "oxk lint is not supported in WASI builds. Use the native npm package or the cargo CLI for linting.",
  ))
}

/// Run the oxlint-compatible linter synchronously.
#[cfg(target_family = "wasm")]
#[napi]
pub fn lint_sync(_args: Vec<String>) -> napi::Result<bool> {
  Err(napi::Error::from_reason(
    "oxk lint is not supported in WASI builds. Use the native npm package or the cargo CLI for linting.",
  ))
}

/// Run the oxlint-compatible linter with JavaScript plugin callbacks.
#[cfg(not(target_family = "wasm"))]
#[napi(
  ts_args_type = "args: Array<string>, loadPlugin: (arg0: string, arg1: string | undefined | null, arg2: boolean, arg3?: string | undefined | null) => Promise<string>, setupRuleConfigs: (arg: string) => string | null, lintFile: (arg0: string, arg1: number, arg2: Uint8Array | undefined | null, arg3: Array<number>, arg4: Array<number>, arg5: string, arg6: string, arg7?: string | undefined | null) => string | null, createWorkspace: (arg: string) => Promise<undefined>, destroyWorkspace: (arg: string) => void, loadJsConfigs: (arg: Array<string>) => Promise<string>"
)]
pub async fn lint_with_plugins(
  args: Vec<String>,
  load_plugin: JsLoadPluginCb,
  setup_rule_configs: JsSetupRuleConfigsCb,
  lint_file: JsLintFileCb,
  create_workspace: JsCreateWorkspaceCb,
  destroy_workspace: JsDestroyWorkspaceCb,
  load_js_configs: JsLoadJsConfigsCb,
) -> bool {
  lint::lint_args_with_plugins(
    args,
    load_plugin,
    setup_rule_configs,
    lint_file,
    create_workspace,
    destroy_workspace,
    load_js_configs,
  )
  .await
}

/// Run the oxlint-compatible linter with JavaScript plugin callbacks.
#[cfg(target_family = "wasm")]
#[napi(
  ts_args_type = "args: Array<string>, loadPlugin: any, setupRuleConfigs: any, lintFile: any, createWorkspace: any, destroyWorkspace: any, loadJsConfigs: any"
)]
pub async fn lint_with_plugins(_args: Vec<String>) -> napi::Result<bool> {
  Err(napi::Error::from_reason(
    "oxk lint with JavaScript plugins is not supported in WASI builds. Use the native npm package for linting.",
  ))
}

fn format_impl(
  filename: String,
  source_text: String,
  options: Option<Value>,
  init_external_formatter_cb: Option<JsInitExternalFormatterCb>,
  format_embedded_cb: Option<JsFormatEmbeddedCb>,
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
    let init_result = ext_fmt.init(num_of_threads);

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

  // Check if external formatter is needed but not provided.
  // JS/TS, TOML, and JSON/JSON5/JSONC are handled by native Rust formatters.
  if !strategy.can_format_without_external() && external_formatter.is_none() {
    return FormatResult {
      code: source_text,
      errors: vec![format!(
        "External formatter is required for file type: {filename}"
      )],
    };
  }

  let resolved_options = config_resolver.resolve(&strategy);

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

/// Format a file with the given options.
///
/// This function supports multiple file types:
/// - JavaScript/TypeScript files (via oxc_formatter)
/// - TOML files (via oxc_toml)
/// - JSON/JSON5/JSONC files (via native Rust formatters)
/// - Other files (via external formatter callbacks when napi feature is enabled)
#[cfg(not(target_family = "wasm"))]
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
  format_impl(
    filename,
    source_text,
    options,
    init_external_formatter_cb,
    format_embedded_cb,
    format_file_cb,
  )
}

#[cfg(target_family = "wasm")]
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
  format_impl(
    filename,
    source_text,
    options,
    init_external_formatter_cb,
    format_embedded_cb,
    format_file_cb,
  )
}
