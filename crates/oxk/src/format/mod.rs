use std::{
    env,
    path::{Path, PathBuf},
    sync::Arc,
};

use format::{
    ConfigResolver, FormatFileStrategy, FormatTargets, SourceFormatter,
    build_global_ignore_matchers, build_ignore_matcher, collect_matching_files, is_gitignore_match,
    resolve_editorconfig_path, resolve_ignore_paths, resolve_oxfmtrc_path, should_ignore_file,
};
use futures::future;
use serde_json::Value;
use tokio::sync::Semaphore;

pub fn run_lsp() -> Result<(), Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .map_err(|e| {
            Box::new(std::io::Error::other(format!(
                "Failed to create tokio runtime: {}",
                e
            ))) as Box<dyn std::error::Error>
        })?;

    runtime.block_on(format::run_lsp(
        "oxfmt".to_string(),
        env!("CARGO_PKG_VERSION").to_string(),
    ));
    Ok(())
}

pub fn format(args: crate::FormatArgs) -> Result<(), Box<dyn std::error::Error>> {
    let patterns = args.file.clone();
    let thread_count = args.thread;
    let excludes = args.excludes.clone();
    let ignore_paths = args.ignore_path.clone();
    let with_node_modules = args.with_node_modules;

    if patterns.is_empty() {
        return Err(Box::new(std::io::Error::other("Missing file pattern")));
    }

    let cwd = env::current_dir().map_err(|e| {
        Box::new(std::io::Error::other(format!(
            "Failed to get current directory: {}",
            e
        ))) as Box<dyn std::error::Error>
    })?;

    // Resolve config (aligned with oxfmt): prefer .oxfmtrc when present, else CLI as Oxfmtrc-like Value
    let oxfmtrc_path = resolve_oxfmtrc_path(&cwd, args.config.as_deref());
    let editorconfig_path = resolve_editorconfig_path(&cwd);

    let mut config_resolver = if oxfmtrc_path.is_some() {
        ConfigResolver::from_config_paths(
            &cwd,
            oxfmtrc_path.as_deref(),
            editorconfig_path.as_deref(),
        )
        .map_err(|e| {
            Box::new(std::io::Error::other(format!(
                "Failed to load configuration: {}",
                e
            ))) as Box<dyn std::error::Error>
        })?
    } else {
        ConfigResolver::from_value(build_value_from_format_args(&args))
    };

    let ignore_patterns = config_resolver.build_and_validate().map_err(|e| {
        Box::new(std::io::Error::other(format!(
            "Failed to parse configuration: {}",
            e
        ))) as Box<dyn std::error::Error>
    })?;

    let config_ignore_root = oxfmtrc_path
        .as_deref()
        .and_then(Path::parent)
        .unwrap_or(&cwd);
    let config_ignore_matcher = build_ignore_matcher(config_ignore_root, &ignore_patterns)
        .map_err(|e| Box::new(std::io::Error::other(e)) as Box<dyn std::error::Error>)?;

    let resolved_ignore_paths = resolve_ignore_paths(&cwd, &ignore_paths)
        .map_err(|e| Box::new(std::io::Error::other(e)) as Box<dyn std::error::Error>)?;
    let targets = FormatTargets::new(&cwd, &patterns, &excludes);
    let global_ignore_matchers =
        build_global_ignore_matchers(&cwd, &targets.exclude_patterns, &resolved_ignore_paths)
            .map_err(|e| Box::new(std::io::Error::other(e)) as Box<dyn std::error::Error>)?;

    let mut files = collect_matching_files(
        &cwd,
        &targets,
        &global_ignore_matchers,
        thread_count,
        with_node_modules,
    )?;

    if let Some(matcher) = &config_ignore_matcher {
        files.retain(|path| !is_gitignore_match(matcher, path, false, true));
    }

    let config_resolver = Arc::new(config_resolver);

    if files.is_empty() {
        return Err(Box::new(std::io::Error::other(
            "No files matched the provided patterns (after ignore rules)",
        )));
    }

    // Create tokio runtime with thread pool size based on thread_count
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(thread_count)
        .enable_all()
        .build()
        .map_err(|e| {
            Box::new(std::io::Error::other(format!(
                "Failed to create tokio runtime: {}",
                e
            ))) as Box<dyn std::error::Error>
        })?;

    // Execute async code in the runtime
    runtime.block_on(async {
        // Create a Semaphore to limit concurrent tasks based on thread_count
        let semaphore = Arc::new(Semaphore::new(thread_count));

        // Spawn a tokio task for each file path
        let mut handles = Vec::new();

        for path in files {
            let semaphore = semaphore.clone();
            let path = path.clone();
            let config_resolver = Arc::clone(&config_resolver);

            // Spawn format_file as a tokio task
            let handle =
                tokio::spawn(
                    async move { format_file_task(path, semaphore, config_resolver).await },
                );
            handles.push(handle);
        }

        // Wait for tasks to complete concurrently
        let mut ast_parse_error = None;
        let mut remaining_handles = handles;

        while !remaining_handles.is_empty() {
            // Select the first completed task
            let (result, _index, remaining) = future::select_all(remaining_handles).await;

            match result {
                Ok(Ok(())) => {
                    // Task completed successfully, continue with remaining tasks
                    remaining_handles = remaining;
                }
                Ok(Err(err)) => {
                    // Check if this is an AST parse error
                    if err.starts_with("AST_PARSE_ERROR:") {
                        // AST parse error: abort all remaining tasks and exit immediately
                        ast_parse_error = Some(err);
                        // Abort all remaining tasks
                        for handle in remaining {
                            handle.abort();
                        }
                        remaining_handles = Vec::new();
                        break;
                    } else {
                        // Non-AST error: print warning and continue processing
                        eprintln!("Warning: {}", err);
                        remaining_handles = remaining;
                    }
                }
                Err(e) => {
                    // Task panicked: treat as fatal error
                    ast_parse_error = Some(format!("Task panicked: {:?}", e));
                    // Abort all remaining tasks
                    for handle in remaining {
                        handle.abort();
                    }
                    remaining_handles = Vec::new();
                    break;
                }
            }
        }

        // Wait for all remaining tasks to finish (including aborted ones)
        for handle in remaining_handles {
            let _ = handle.await;
        }

        // Return error only if AST parse error occurred
        if let Some(err) = ast_parse_error {
            // Remove the prefix when returning the error
            let error_msg = if err.starts_with("AST_PARSE_ERROR:") {
                err.strip_prefix("AST_PARSE_ERROR: ")
                    .unwrap_or(&err)
                    .to_string()
            } else {
                err
            };
            return Err(Box::new(std::io::Error::other(error_msg)) as Box<dyn std::error::Error>);
        }

        Ok(())
    })
}

/// Build an Oxfmtrc-like JSON Value from CLI FormatArgs (used when no .oxfmtrc is found).
/// Aligned with oxfmt's FormatConfig / Oxfmtrc camelCase keys.
fn build_value_from_format_args(args: &crate::FormatArgs) -> Value {
    let mut m = serde_json::Map::new();
    if let Some(v) = &args.indent_style {
        m.insert("useTabs".into(), Value::Bool(v.is_tab()));
    }
    if let Some(v) = &args.indent_width {
        m.insert("tabWidth".into(), Value::from(v.value()));
    }
    if let Some(v) = &args.line_ending {
        m.insert(
            "endOfLine".into(),
            Value::String(format!("{:?}", v).to_lowercase()),
        );
    }
    if let Some(v) = &args.line_width {
        m.insert("printWidth".into(), Value::from(v.value()));
    }
    if let Some(v) = &args.quote_style {
        m.insert("singleQuote".into(), Value::Bool(!v.is_double()));
    }
    if let Some(v) = &args.jsx_quote_style {
        m.insert("jsxSingleQuote".into(), Value::Bool(!v.is_double()));
    }
    if let Some(v) = &args.trailing_commas {
        m.insert(
            "trailingComma".into(),
            Value::String(format!("{:?}", v).to_lowercase()),
        );
    }
    if let Some(v) = &args.semicolons {
        m.insert("semi".into(), Value::Bool(v.is_always()));
    }
    if let Some(v) = &args.arrow_parentheses {
        m.insert(
            "arrowParens".into(),
            Value::String(if v.is_always() { "always" } else { "avoid" }.to_string()),
        );
    }
    if let Some(v) = &args.bracket_spacing {
        m.insert("bracketSpacing".into(), Value::Bool(v.value()));
    }
    if let Some(v) = &args.bracket_same_line {
        m.insert("bracketSameLine".into(), Value::Bool(v.value()));
    }
    if let Some(v) = &args.attribute_position {
        m.insert(
            "singleAttributePerLine".into(),
            Value::Bool(matches!(v, oxc_formatter::AttributePosition::Multiline)),
        );
    }
    if let Some(v) = &args.expand {
        let s = match v {
            oxc_formatter::Expand::Auto => "preserve",
            _ => "collapse",
        };
        m.insert("objectWrap".into(), Value::String(s.to_string()));
    }
    if let Some(v) = &args.embedded_language_formatting {
        m.insert(
            "embeddedLanguageFormatting".into(),
            Value::String(format!("{:?}", v).to_lowercase()),
        );
    }
    Value::Object(m)
}

/// Format a single file as a tokio task
async fn format_file_task(
    path: PathBuf,
    semaphore: Arc<Semaphore>,
    config_resolver: Arc<ConfigResolver>,
) -> Result<(), String> {
    // Acquire permit to limit concurrency
    let _permit = semaphore
        .acquire()
        .await
        .map_err(|e| format!("Semaphore error: {}", e))?;

    // Use async file I/O for better performance in concurrent scenarios
    format_file_async(&path, config_resolver)
        .await
        .map_err(|err| format!("{}: {err}", path.display()))
}

/// Format a single file using async I/O (config resolution aligned with oxfmt)
async fn format_file_async(
    path: &Path,
    config_resolver: Arc<ConfigResolver>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Verify file exists
    let actual_path = if tokio::fs::metadata(path).await.is_ok() {
        path.to_path_buf()
    } else {
        return Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("File '{}' does not exist", path.display()),
        )));
    };

    // Read the file using async I/O
    let bytes = tokio::fs::read(&actual_path)
        .await
        .map_err(|e| format!("Failed to read file '{}': {}", actual_path.display(), e))?;

    let source_text = String::from_utf8_lossy(&bytes).into_owned();

    // Skip empty files silently
    if source_text.is_empty() {
        return Ok(());
    }

    // Skip ignored files silently (e.g., lock files, ignored JSON files)
    if should_ignore_file(&actual_path) {
        return Ok(());
    }

    // Determine format strategy from file path
    let strategy = FormatFileStrategy::try_from(actual_path.clone())
        .map_err(|_| format!("Unsupported file type '{}'", actual_path.display()))?;

    // Reject ExternalFormatter: oxk CLI has no napi/Prettier (aligned with oxfmt's Mode::Cli behavior)
    if let FormatFileStrategy::ExternalFormatter { parser_name, .. }
    | FormatFileStrategy::ExternalFormatterPackageJson { parser_name, .. } = &strategy
    {
        return Err(format!(
            "File type '{}' (parser: {}) requires external formatter support (e.g., Prettier). \
            oxk CLI only supports JavaScript/TypeScript, TOML, and JSON/JSON5/JSONC files. \
            For other file types, please use npm/oxk with external formatter callbacks or use a different formatter.",
            actual_path.display(),
            parser_name
        )
        .into());
    }

    // Resolve options from ConfigResolver (aligned with oxfmt: .oxfmtrc / overrides / editorconfig)
    let resolved_options = config_resolver.resolve(&strategy);

    // Run CPU-intensive parsing and formatting in a blocking task
    let actual_path_clone = actual_path.clone();
    let strategy_clone = match &strategy {
        FormatFileStrategy::OxcFormatter { path, source_type } => {
            FormatFileStrategy::OxcFormatter {
                path: path.clone(),
                source_type: *source_type,
            }
        }
        FormatFileStrategy::OxfmtToml { path } => {
            FormatFileStrategy::OxfmtToml { path: path.clone() }
        }
        FormatFileStrategy::OxfmtJson { path, json_type } => FormatFileStrategy::OxfmtJson {
            path: path.clone(),
            json_type: *json_type,
        },
        FormatFileStrategy::ExternalFormatter { .. }
        | FormatFileStrategy::ExternalFormatterPackageJson { .. } => {
            // This should never happen as we check earlier in resolved_options match
            unreachable!("ExternalFormatter should be rejected earlier")
        }
    };
    let formatted_code = tokio::task::spawn_blocking(move || {
        // Create formatter
        let formatter = SourceFormatter::new(1);

        // Format the file
        match formatter.format(&strategy_clone, &source_text, resolved_options) {
            format::FormatResult::Success { code, .. } => {
                // Check for parse errors by comparing with original
                // If there were parse errors, the formatter would have returned an error
                Ok(code)
            }
            format::FormatResult::Error(diagnostics) => {
                // Format parse/format errors
                let mut error_msg = format!(
                    "AST_PARSE_ERROR: Parser errors in '{}':\n",
                    actual_path_clone.display()
                );
                for diagnostic in diagnostics {
                    error_msg.push_str(&format!("{diagnostic:?}\n"));
                }
                Err(error_msg)
            }
        }
    })
    .await
    .map_err(|e| format!("Task join error: {}", e))?
    .map_err(|e| Box::new(std::io::Error::other(e)) as Box<dyn std::error::Error>)?;

    // Write back to the actual path using async I/O
    tokio::fs::write(&actual_path, formatted_code)
        .await
        .map_err(|_| format!("Failed to write to '{}'", actual_path.display()).into())
}

#[cfg(test)]
mod tests {
    use format::{FormatFileStrategy, ResolvedOptions, SourceFormatter};
    use oxc_formatter::JsFormatOptions as FormatOptions;
    use serde_json::Value;
    use std::{
        fs,
        path::{Path, PathBuf},
        process,
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::{
        FormatTargets, build_global_ignore_matchers, collect_matching_files, resolve_ignore_paths,
    };

    static NEXT_TEST_DIR_ID: AtomicU64 = AtomicU64::new(0);

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(prefix: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "oxc-ark-format-{prefix}-{}-{}",
                process::id(),
                NEXT_TEST_DIR_ID.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).expect("test temp dir should be created");
            let path = path
                .canonicalize()
                .expect("test temp dir should canonicalize");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }

        fn join(&self, child: &str) -> PathBuf {
            self.path.join(child)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn relative_files(paths: Vec<PathBuf>, root: &Path) -> Vec<String> {
        paths
            .into_iter()
            .map(|path| {
                path.strip_prefix(root)
                    .expect("path should be under temp root")
                    .to_string_lossy()
                    .to_string()
            })
            .collect()
    }

    fn format_code(path: &str, source: &str) -> Result<String, String> {
        let strategy = FormatFileStrategy::try_from(PathBuf::from(path))
            .map_err(|_| format!("Unsupported file type: {}", path))?;

        let format_options = match &strategy {
            FormatFileStrategy::OxcFormatter { .. } => FormatOptions::default(),
            _ => return Err("Only JS/TS files supported in tests".to_string()),
        };

        let formatter = SourceFormatter::new(1);
        let resolved_options = ResolvedOptions::OxcFormatter {
            format_options: Box::new(format_options),
            external_options: Value::Object(serde_json::Map::new()),
            insert_final_newline: true,
        };

        match formatter.format(&strategy, source, resolved_options) {
            format::FormatResult::Success { code, .. } => Ok(code),
            format::FormatResult::Error(diagnostics) => {
                Err(format!("Format errors: {:?}", diagnostics))
            }
        }
    }

    #[test]
    fn collect_matching_files_respects_prettierignore() {
        let dir = TestDir::new("prettierignore");
        fs::create_dir_all(dir.join("dist")).expect("dist dir should be created");
        fs::write(dir.join("src.ts"), "const a=1\n").expect("src file should be written");
        fs::write(dir.join("dist/ignored.ts"), "const b=1\n")
            .expect("ignored file should be written");
        fs::write(dir.join(".prettierignore"), "dist\n").expect("ignore file should be written");

        let targets = FormatTargets::new(dir.path(), &[".".to_string()], &[]);
        let ignore_paths =
            resolve_ignore_paths(dir.path(), &[]).expect("default ignore path should resolve");
        let global_ignores =
            build_global_ignore_matchers(dir.path(), &targets.exclude_patterns, &ignore_paths)
                .expect("ignore matcher should build");

        let files = collect_matching_files(dir.path(), &targets, &global_ignores, 1, false)
            .expect("files should collect");

        assert_eq!(relative_files(files, dir.path()), vec!["src.ts"]);
    }

    #[test]
    fn collect_matching_files_supports_oxfmt_globs_and_bang_excludes() {
        let dir = TestDir::new("glob-exclude");
        fs::create_dir_all(dir.join("src/generated")).expect("dirs should be created");
        fs::write(dir.join("src/main.ts"), "const a=1\n").expect("main should be written");
        fs::write(dir.join("src/generated/main.ts"), "const b=1\n")
            .expect("generated should be written");

        let targets = FormatTargets::new(
            dir.path(),
            &["src/**/*.ts".to_string(), "!src/generated".to_string()],
            &[],
        );
        let global_ignores =
            build_global_ignore_matchers(dir.path(), &targets.exclude_patterns, &[])
                .expect("ignore matcher should build");

        let files = collect_matching_files(dir.path(), &targets, &global_ignores, 1, false)
            .expect("files should collect");

        assert_eq!(relative_files(files, dir.path()), vec!["src/main.ts"]);
    }

    #[test]
    fn collect_matching_files_skips_package_modules_by_default() {
        let dir = TestDir::new("package-modules");
        fs::create_dir_all(dir.join("node_modules/pkg")).expect("dirs should be created");
        fs::create_dir_all(dir.join("oh_modules/pkg")).expect("dirs should be created");
        fs::write(dir.join("node_modules/pkg/index.ts"), "const a=1\n")
            .expect("node module file should be written");
        fs::write(dir.join("oh_modules/pkg/index.ts"), "const b=1\n")
            .expect("oh module file should be written");
        fs::write(dir.join("index.ts"), "const c=1\n").expect("index should be written");

        let targets = FormatTargets::new(dir.path(), &[".".to_string()], &[]);
        let files = collect_matching_files(dir.path(), &targets, &[], 1, false)
            .expect("files should collect");

        assert_eq!(relative_files(files, dir.path()), vec!["index.ts"]);

        let files = collect_matching_files(dir.path(), &targets, &[], 1, true)
            .expect("files should collect");

        assert_eq!(
            relative_files(files, dir.path()),
            vec![
                "index.ts",
                "node_modules/pkg/index.ts",
                "oh_modules/pkg/index.ts"
            ]
        );
    }

    #[test]
    fn test_format_arkts_file() {
        let source = r#"@Component
struct MyComponent {
  @State message: string = 'Hello World'
  @State count: number = 0

  build() {
    Row() {
      Column() {
        Text(this.message)
          .fontSize(20)
          .fontWeight(FontWeight.Bold)
        Button('Click me')
          .onClick(() => {
            this.count++
          })
      }
      .width('100%')
    }
    .height('100%')
  }
}"#;

        let result = format_code("test.ets", source);
        assert!(result.is_ok(), "ArkTS file should format successfully");
        let formatted = result.expect("Format should succeed in test");
        assert!(!formatted.is_empty(), "Formatted code should not be empty");
        // Verify the formatted code contains key ArkTS elements
        assert!(
            formatted.contains("@Component"),
            "Should contain @Component"
        );
        assert!(formatted.contains("struct"), "Should contain struct");
    }

    #[test]
    fn test_format_arkts_with_complex_syntax() {
        let source = r#"@Entry
@Component
struct Index {
  @State message: string = 'Hello ArkUI'
  private data: Array<string> = ['item1', 'item2', 'item3']

  aboutToAppear() {
    console.log('Component about to appear')
  }

  build() {
    Column({ space: 20 }) {
      Text(this.message)
        .fontSize(30)
        .fontColor(Color.Blue)
      ForEach(this.data, (item: string, index: number) => {
        Text(item)
          .fontSize(16)
      })
    }
    .padding(20)
    .width('100%')
    .height('100%')
  }
}"#;

        let result = format_code("index.ets", source);
        assert!(
            result.is_ok(),
            "Complex ArkTS file should format successfully"
        );
    }

    #[test]
    fn test_format_json5_file_strategy() {
        // Test that JSON5 files are recognized as OxfmtJson
        let path = PathBuf::from("test.json5");
        let strategy = FormatFileStrategy::try_from(path);

        match strategy {
            Ok(FormatFileStrategy::OxfmtJson { json_type, .. }) => {
                use format::JsonType;
                assert_eq!(
                    json_type,
                    JsonType::Json5,
                    "JSON5 files should use Json5 type"
                );
            }
            Ok(other) => {
                panic!(
                    "JSON5 files should be recognized as OxfmtJson, got: {:?}",
                    format!("{:?}", other)
                );
            }
            Err(_) => {
                panic!("JSON5 files should be recognized");
            }
        }
    }

    #[test]
    fn test_format_json5_content() {
        let json5_content = r#"{
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

        // Test that JSON5 files can be formatted using Rust formatter
        let path = PathBuf::from("package.json5");
        let strategy =
            FormatFileStrategy::try_from(path.clone()).expect("JSON5 file should be recognized");

        // Verify it's OxfmtJson
        match &strategy {
            FormatFileStrategy::OxfmtJson { json_type, .. } => {
                use format::JsonType;
                assert_eq!(*json_type, JsonType::Json5);
            }
            _ => panic!("JSON5 file should be recognized as OxfmtJson"),
        }

        // Test actual formatting
        let formatter = SourceFormatter::new(1);
        let resolved_options = ResolvedOptions::OxfmtJson {
            json_options: format::JsonFormatterOptions {
                indent_width: 2,
                use_tabs: false,
                line_ending: "\n".to_string(),
                trailing_commas: false,
                quote_properties: json5format::QuoteProperties::Consistent,
            },
            json_type: format::JsonType::Json5,
            insert_final_newline: true,
        };

        match formatter.format(&strategy, json5_content, resolved_options) {
            format::FormatResult::Success { code, .. } => {
                assert!(!code.is_empty(), "Formatted JSON5 should not be empty");
                // Verify the formatted code contains key elements
                assert!(code.contains("name"), "Should contain 'name'");
                assert!(code.contains("test"), "Should contain 'test'");
            }
            format::FormatResult::Error(diagnostics) => {
                panic!(
                    "JSON5 formatting should succeed, got errors: {:?}",
                    diagnostics
                );
            }
        }
    }

    #[test]
    fn test_format_json_file() {
        let json_content = r#"{"name":"test","version":"1.0.0","description":"Test package"}"#;

        let path = PathBuf::from("test.json");
        let strategy =
            FormatFileStrategy::try_from(path.clone()).expect("JSON file should be recognized");

        // Verify it's OxfmtJson
        match &strategy {
            FormatFileStrategy::OxfmtJson { json_type, .. } => {
                use format::JsonType;
                assert_eq!(*json_type, JsonType::Json);
            }
            _ => panic!("JSON file should be recognized as OxfmtJson"),
        }

        // Test actual formatting
        let formatter = SourceFormatter::new(1);
        let resolved_options = ResolvedOptions::OxfmtJson {
            json_options: format::JsonFormatterOptions {
                indent_width: 2,
                use_tabs: false,
                line_ending: "\n".to_string(),
                trailing_commas: false,
                quote_properties: json5format::QuoteProperties::Consistent,
            },
            json_type: format::JsonType::Json,
            insert_final_newline: true,
        };

        match formatter.format(&strategy, json_content, resolved_options) {
            format::FormatResult::Success { code, .. } => {
                assert!(!code.is_empty(), "Formatted JSON should not be empty");
                assert!(code.contains("name"), "Should contain 'name'");
            }
            format::FormatResult::Error(diagnostics) => {
                panic!(
                    "JSON formatting should succeed, got errors: {:?}",
                    diagnostics
                );
            }
        }
    }

    #[test]
    fn test_format_jsonc_file() {
        let jsonc_content = r#"{
  // This is a comment
  "name": "test",
  "version": "1.0.0",
  /* Another comment */
  "description": "Test package"
}"#;

        let path = PathBuf::from("test.jsonc");
        let strategy =
            FormatFileStrategy::try_from(path.clone()).expect("JSONC file should be recognized");

        // Verify it's OxfmtJson
        match &strategy {
            FormatFileStrategy::OxfmtJson { json_type, .. } => {
                use format::JsonType;
                assert_eq!(*json_type, JsonType::Jsonc);
            }
            _ => panic!("JSONC file should be recognized as OxfmtJson"),
        }

        // Test actual formatting
        let formatter = SourceFormatter::new(1);
        let resolved_options = ResolvedOptions::OxfmtJson {
            json_options: format::JsonFormatterOptions {
                indent_width: 2,
                use_tabs: false,
                line_ending: "\n".to_string(),
                trailing_commas: false,
                quote_properties: json5format::QuoteProperties::Consistent,
            },
            json_type: format::JsonType::Jsonc,
            insert_final_newline: true,
        };

        match formatter.format(&strategy, jsonc_content, resolved_options) {
            format::FormatResult::Success { code, .. } => {
                assert!(!code.is_empty(), "Formatted JSONC should not be empty");
                // Comments should be stripped
                assert!(
                    !code.contains("//"),
                    "Comments should be stripped from JSONC"
                );
                assert!(
                    !code.contains("/*"),
                    "Comments should be stripped from JSONC"
                );
                assert!(code.contains("name"), "Should contain 'name'");
            }
            format::FormatResult::Error(diagnostics) => {
                panic!(
                    "JSONC formatting should succeed, got errors: {:?}",
                    diagnostics
                );
            }
        }
    }
}
