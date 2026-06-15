#![deny(clippy::all)]

mod parse;

use napi_derive::napi;
use serde_json::Value;
#[cfg(not(target_family = "wasm"))]
use std::ffi::OsString;
use std::path::PathBuf;
#[cfg(not(target_family = "wasm"))]
use std::{env, fs, path::Path, sync::Arc};

#[cfg(not(target_family = "wasm"))]
use format::{
  build_global_ignore_matchers, build_ignore_matcher, collect_matching_files, is_gitignore_match,
  resolve_editorconfig_path, resolve_ignore_paths, resolve_oxfmtrc_path, FormatTargets,
};
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
#[napi(object)]
pub struct FormatFilesArgs {
  pub patterns: Vec<String>,
  pub excludes: Vec<String>,
  pub ignore_paths: Vec<String>,
  pub with_node_modules: bool,
  pub thread_count: u32,
  pub config_path: Option<String>,
  pub cli_options: Option<Value>,
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

/// Run the oxfmt-compatible formatter over files.
#[cfg(not(target_family = "wasm"))]
#[napi]
pub async fn format_files(
  args: FormatFilesArgs,
  #[napi(ts_arg_type = "(numThreads: number) => Promise<string[]>")]
  init_external_formatter_cb: JsInitExternalFormatterCb,
  #[napi(
    ts_arg_type = "(options: Record<string, any>, tagName: string, code: string) => Promise<string>"
  )]
  format_embedded_cb: JsFormatEmbeddedCb,
  #[napi(
    ts_arg_type = "(options: Record<string, any>, parserName: string, fileName: string, code: string) => Promise<string>"
  )]
  format_file_cb: JsFormatFileCb,
) -> napi::Result<bool> {
  let external_formatter = ExternalFormatter::new(
    init_external_formatter_cb,
    format_embedded_cb,
    format_file_cb,
  );
  format_files_impl(args, external_formatter)
    .await
    .map_err(napi::Error::from_reason)
}

#[cfg(target_family = "wasm")]
#[napi]
pub async fn format_files(_args: Value) -> napi::Result<bool> {
  Err(napi::Error::from_reason(
    "oxk format files is not supported in WASI builds. Use the native npm package or the cargo CLI.",
  ))
}

#[cfg(not(target_family = "wasm"))]
async fn format_files_impl(
  args: FormatFilesArgs,
  external_formatter: ExternalFormatter,
) -> Result<bool, String> {
  if args.patterns.is_empty() {
    return Err("Missing file pattern".to_string());
  }

  let cwd = env::current_dir().map_err(|err| format!("Failed to get current directory: {err}"))?;
  let thread_count = usize::try_from(args.thread_count.max(1)).unwrap_or(1);
  let config_path = args.config_path.as_deref().map(PathBuf::from);
  let oxfmtrc_path = resolve_oxfmtrc_path(&cwd, config_path.as_deref());
  let editorconfig_path = resolve_editorconfig_path(&cwd);

  let mut config_resolver = if oxfmtrc_path.is_some() {
    ConfigResolver::from_config_paths(&cwd, oxfmtrc_path.as_deref(), editorconfig_path.as_deref())
      .map_err(|err| format!("Failed to load configuration: {err}"))?
  } else {
    ConfigResolver::from_value(args.cli_options.unwrap_or_else(empty_object))
  };

  let ignore_patterns = config_resolver
    .build_and_validate()
    .map_err(|err| format!("Failed to parse configuration: {err}"))?;

  let config_ignore_root = oxfmtrc_path
    .as_deref()
    .and_then(Path::parent)
    .unwrap_or(&cwd);
  let config_ignore_matcher = build_ignore_matcher(config_ignore_root, &ignore_patterns)?;

  let ignore_paths = args
    .ignore_paths
    .iter()
    .map(PathBuf::from)
    .collect::<Vec<_>>();
  let resolved_ignore_paths = resolve_ignore_paths(&cwd, &ignore_paths)?;
  let targets = FormatTargets::new(&cwd, &args.patterns, &args.excludes);
  let global_ignore_matchers =
    build_global_ignore_matchers(&cwd, &targets.exclude_patterns, &resolved_ignore_paths)?;

  let mut files = collect_matching_files(
    &cwd,
    &targets,
    &global_ignore_matchers,
    thread_count,
    args.with_node_modules,
  )
  .map_err(|err| err.to_string())?;

  if let Some(matcher) = &config_ignore_matcher {
    files.retain(|path| !is_gitignore_match(matcher, path, false, true));
  }

  if files.is_empty() {
    return Err("No files matched the provided patterns (after ignore rules)".to_string());
  }

  tokio::task::block_in_place(|| external_formatter.init(thread_count))?;

  let config_resolver = Arc::new(config_resolver);
  let semaphore = Arc::new(tokio::sync::Semaphore::new(thread_count));
  let mut handles = Vec::with_capacity(files.len());

  for path in files {
    let cwd = cwd.clone();
    let config_resolver = Arc::clone(&config_resolver);
    let external_formatter = external_formatter.clone();
    let semaphore = Arc::clone(&semaphore);
    handles.push(tokio::spawn(async move {
      format_cli_file(path, cwd, config_resolver, external_formatter, semaphore).await
    }));
  }

  let mut formatted_count = 0usize;
  let mut success = true;
  for handle in handles {
    match handle.await {
      Ok(Ok(formatted)) => {
        if formatted {
          formatted_count += 1;
        }
      }
      Ok(Err(err)) => {
        eprintln!("Error formatting {err}");
        success = false;
      }
      Err(err) => {
        eprintln!("Error formatting task panicked: {err}");
        success = false;
      }
    }
  }

  println!("\nFormatted {formatted_count} file(s)");
  Ok(success)
}

#[cfg(not(target_family = "wasm"))]
async fn format_cli_file(
  path: PathBuf,
  cwd: PathBuf,
  config_resolver: Arc<ConfigResolver>,
  external_formatter: ExternalFormatter,
  semaphore: Arc<tokio::sync::Semaphore>,
) -> Result<bool, String> {
  let _permit = semaphore
    .acquire()
    .await
    .map_err(|err| format!("{}: semaphore error: {err}", path.display()))?;

  tokio::task::spawn_blocking(move || {
    format_cli_file_blocking(path, &cwd, &config_resolver, external_formatter)
  })
  .await
  .map_err(|err| format!("task join error: {err}"))?
}

#[cfg(not(target_family = "wasm"))]
fn format_cli_file_blocking(
  path: PathBuf,
  cwd: &Path,
  config_resolver: &ConfigResolver,
  external_formatter: ExternalFormatter,
) -> Result<bool, String> {
  let source_text = fs::read_to_string(&path)
    .map_err(|err| format!("{}: failed to read file: {err}", path.display()))?;
  if source_text.is_empty() || should_ignore_file(&path) {
    return Ok(false);
  }

  let strategy = FormatFileStrategy::try_from(path.clone())
    .map_err(|_| format!("{}: unsupported file type", path.display()))?;
  let resolved_options = config_resolver.resolve(&strategy);
  let formatter = SourceFormatter::new(1).with_external_formatter(Some(external_formatter));

  let formatted_code = match formatter.format(&strategy, &source_text, resolved_options) {
    CoreFormatResult::Success { code, .. } => code,
    CoreFormatResult::Error(diagnostics) => {
      let errors = diagnostics
        .iter()
        .map(|diagnostic| format!("{diagnostic}"))
        .collect::<Vec<_>>()
        .join("\n");
      return Err(format!("{}:\n{errors}", path.display()));
    }
  };

  fs::write(&path, formatted_code)
    .map_err(|err| format!("{}: failed to write file: {err}", path.display()))?;
  let display_path = path.strip_prefix(cwd).unwrap_or(&path);
  println!("Formatted: {}", display_path.display());
  Ok(true)
}

fn empty_object() -> Value {
  Value::Object(serde_json::Map::new())
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
