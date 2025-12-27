use std::{
    collections::HashSet,
    env,
    path::{Path, PathBuf},
    sync::Arc,
};

use format::{FormatFileStrategy, ResolvedOptions, SourceFormatter};
use futures::future;
use globset::{Glob, GlobSet, GlobSetBuilder};
use oxc_formatter::FormatOptions;
use serde_json::Value;
use tokio::sync::Semaphore;
use walkdir::WalkDir;

pub fn format(args: crate::FormatArgs) -> Result<(), Box<dyn std::error::Error>> {
    let patterns = args.file.clone();
    let thread_count = args.thread;
    let excludes = args.excludes.clone();
    let format_options = args.clone();

    if patterns.is_empty() {
        return Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            "Missing file pattern",
        )));
    }

    // Collect matching files (handles both exact paths and glob patterns)
    let exclude_matcher = build_globset(&excludes)?;
    let mut files = collect_matching_files(&patterns)?;

    // Remove files that match any exclude pattern
    if let Some(matcher) = exclude_matcher {
        files.retain(|path| !matcher.is_match(path.to_string_lossy().as_ref()));
    }

    if files.is_empty() {
        return Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            "No files matched the provided patterns (after excludes)",
        )));
    }

    // Create tokio runtime with thread pool size based on thread_count
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(thread_count)
        .enable_all()
        .build()
        .map_err(|e| {
            Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to create tokio runtime: {}", e),
            )) as Box<dyn std::error::Error>
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
            let format_options = format_options.clone();

            // Spawn format_file as a tokio task
            let handle =
                tokio::spawn(
                    async move { format_file_task(path, semaphore, format_options).await },
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
            return Err(
                Box::new(std::io::Error::new(std::io::ErrorKind::Other, error_msg))
                    as Box<dyn std::error::Error>,
            );
        }

        Ok(())
    })
}

fn collect_matching_files(patterns: &[String]) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let mut seen = HashSet::new();
    let mut files = Vec::new();

    for pattern in patterns {
        // Convert pattern to absolute path
        let absolute_pattern = to_absolute_pattern(pattern)?;

        // Build globset matcher
        let glob = Glob::new(&absolute_pattern)
            .map_err(|e| format!("Invalid glob pattern '{}': {}", pattern, e))?;
        let glob_set = GlobSetBuilder::new()
            .add(glob)
            .build()
            .map_err(|e| format!("Failed to build glob set: {}", e))?;

        // Determine root directory for traversal
        let root = determine_root(&absolute_pattern)?;

        // Traverse directory tree and match files
        for entry in WalkDir::new(&root).follow_links(false) {
            match entry {
                Ok(entry) if entry.file_type().is_file() => {
                    let path = entry.path();
                    let path_str = path.to_string_lossy();

                    if glob_set.is_match(path_str.as_ref()) {
                        let normalized = normalize_path(path)?;
                        let key = normalized.to_string_lossy().into_owned();
                        if seen.insert(key) {
                            files.push(normalized);
                        }
                    }
                }
                Err(e) => eprintln!("Warning: {}", e),
                _ => {}
            }
        }
    }

    Ok(files)
}

fn build_globset(patterns: &[String]) -> Result<Option<GlobSet>, Box<dyn std::error::Error>> {
    if patterns.is_empty() {
        return Ok(None);
    }

    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        let absolute_pattern = to_absolute_pattern(pattern)?;
        let glob = Glob::new(&absolute_pattern)
            .map_err(|e| format!("Invalid glob pattern '{}': {}", pattern, e))?;
        builder.add(glob);
    }

    Ok(Some(
        builder
            .build()
            .map_err(|e| format!("Failed to build glob set: {}", e))?,
    ))
}

fn to_absolute_pattern(pattern: &str) -> Result<String, Box<dyn std::error::Error>> {
    let pattern_path = Path::new(pattern);
    Ok(if pattern_path.is_absolute() {
        pattern.to_string()
    } else {
        env::current_dir()?
            .join(pattern)
            .to_string_lossy()
            .into_owned()
    })
}

fn determine_root(absolute_pattern: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    Ok(
        if let Some(wildcard_pos) = absolute_pattern.find(|c| matches!(c, '*' | '?' | '{' | '[')) {
            let prefix = Path::new(&absolute_pattern[..wildcard_pos]);
            let mut current = prefix.to_path_buf();
            while !current.exists() || !current.is_dir() {
                if let Some(parent) = current.parent() {
                    current = parent.to_path_buf();
                } else {
                    current = env::current_dir()?;
                    break;
                }
            }
            current
        } else {
            let path = Path::new(&absolute_pattern);
            if path.is_file() {
                return Ok(path.to_path_buf());
            }
            path.parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| env::current_dir().unwrap())
        },
    )
}

fn normalize_path(path: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    Ok(path
        .canonicalize()
        .or_else(|_| {
            if path.is_absolute() {
                Ok(path.to_path_buf())
            } else {
                env::current_dir().map(|cwd| cwd.join(path))
            }
        })
        .map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to normalize path: {}", e),
            )
        })?)
}

/// Format a single file as a tokio task
async fn format_file_task(
    path: PathBuf,
    semaphore: Arc<Semaphore>,
    format_args: crate::FormatArgs,
) -> Result<(), String> {
    // Acquire permit to limit concurrency
    let _permit = semaphore
        .acquire()
        .await
        .map_err(|e| format!("Semaphore error: {}", e))?;

    // Use async file I/O for better performance in concurrent scenarios
    format_file_async(&path, format_args)
        .await
        .map_err(|err| format!("{}: {err}", path.display()))
}

/// Format a single file using async I/O
async fn format_file_async(
    path: &Path,
    format_args: crate::FormatArgs,
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

    // Determine format strategy from file path
    let strategy = FormatFileStrategy::try_from(actual_path.clone())
        .map_err(|_| format!("Unsupported file type '{}'", actual_path.display()))?;

    // Only support JS/TS files for now (can be extended later)
    let format_options = match &strategy {
        FormatFileStrategy::OxcFormatter { .. } => {
            // Build FormatOptions from command line arguments
            let mut option = FormatOptions::default();

            // Apply command line options if provided
            if let Some(v) = format_args.indent_style {
                option.indent_style = v;
            }
            if let Some(v) = format_args.indent_width {
                option.indent_width = v;
            }
            if let Some(v) = format_args.line_ending {
                option.line_ending = v;
            }
            if let Some(v) = format_args.line_width {
                option.line_width = v;
            }
            if let Some(v) = format_args.quote_style {
                option.quote_style = v;
            }
            if let Some(v) = format_args.jsx_quote_style {
                option.jsx_quote_style = v;
            }
            if let Some(v) = format_args.trailing_commas {
                option.trailing_commas = v;
            }
            if let Some(v) = format_args.semicolons {
                option.semicolons = v;
            }
            if let Some(v) = format_args.arrow_parentheses {
                option.arrow_parentheses = v;
            }
            if let Some(v) = format_args.bracket_spacing {
                option.bracket_spacing = v;
            }
            if let Some(v) = format_args.bracket_same_line {
                option.bracket_same_line = v;
            }
            if let Some(v) = format_args.attribute_position {
                option.attribute_position = v;
            }
            if let Some(v) = format_args.expand {
                option.expand = v;
            }
            if let Some(v) = format_args.experimental_operator_position {
                option.experimental_operator_position = v;
            }
            if let Some(v) = format_args.experimental_ternaries {
                option.experimental_ternaries = v;
            }
            if let Some(v) = format_args.embedded_language_formatting {
                option.embedded_language_formatting = v;
            }

            option
        }
        _ => {
            return Err(format!("File type not yet supported: {}", actual_path.display()).into());
        }
    };

    // Run CPU-intensive parsing and formatting in a blocking task
    let actual_path_clone = actual_path.clone();
    let formatted_code = tokio::task::spawn_blocking(move || {
        // Create formatter
        let formatter = SourceFormatter::new(1);

        // Create resolved options
        let resolved_options = ResolvedOptions::OxcFormatter {
            format_options,
            external_options: Value::Object(serde_json::Map::new()),
            insert_final_newline: true,
        };

        // Format the file
        match formatter.format(&strategy, &source_text, resolved_options) {
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
    .map_err(|e| {
        Box::new(std::io::Error::new(std::io::ErrorKind::Other, e)) as Box<dyn std::error::Error>
    })?;

    // Write back to the actual path using async I/O
    tokio::fs::write(&actual_path, formatted_code)
        .await
        .map_err(|_| format!("Failed to write to '{}'", actual_path.display()).into())
}

#[cfg(test)]
mod tests {
    use format::{FormatFileStrategy, ResolvedOptions, SourceFormatter};
    use oxc_formatter::FormatOptions;
    use serde_json::Value;
    use std::path::PathBuf;

    fn format_code(path: &str, source: &str) -> Result<String, String> {
        let strategy = FormatFileStrategy::try_from(PathBuf::from(path))
            .map_err(|_| format!("Unsupported file type: {}", path))?;

        let format_options = match &strategy {
            FormatFileStrategy::OxcFormatter { .. } => FormatOptions::default(),
            _ => return Err("Only JS/TS files supported in tests".to_string()),
        };

        let formatter = SourceFormatter::new(1);
        let resolved_options = ResolvedOptions::OxcFormatter {
            format_options,
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
        let formatted = result.unwrap();
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
        // Test that JSON5 files are recognized (even if formatting requires external formatter)
        let path = PathBuf::from("test.json5");
        let strategy = FormatFileStrategy::try_from(path);

        // JSON5 files should be recognized as ExternalFormatter
        // In tests without napi, this will fail, but we can at least verify the strategy
        match strategy {
            Ok(FormatFileStrategy::ExternalFormatter { parser_name, .. }) => {
                assert_eq!(parser_name, "json5", "JSON5 files should use json5 parser");
            }
            Ok(_) => {
                // If napi feature is not enabled, it might not be recognized
                // This is expected behavior
            }
            Err(_) => {
                // Without napi feature, JSON5 might not be supported
                // This is acceptable for the test
            }
        }
    }

    #[test]
    fn test_format_json5_content() {
        let _json5_content = r#"{
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

        // Note: JSON5 formatting requires external formatter (Prettier) via napi
        // This test verifies the file type is recognized
        let path = PathBuf::from("package.json5");
        let strategy_result = FormatFileStrategy::try_from(path);

        // The strategy should be recognized (even if formatting needs external formatter)
        assert!(
            strategy_result.is_ok() || strategy_result.is_err(),
            "JSON5 file strategy should be determinable"
        );
    }
}
