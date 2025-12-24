use std::{
    collections::HashSet,
    env,
    path::{Path, PathBuf},
    sync::Arc,
};

use futures::future;
use globset::{Glob, GlobSet, GlobSetBuilder};
use oxc_allocator::Allocator;
use oxc_formatter::{FormatOptions, Formatter, QuoteProperties, get_parse_options};
use oxc_parser::Parser;
use oxc_span::SourceType;
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
    // block_on will wait for the future to complete, but we need to ensure all spawned tasks complete
    runtime.block_on(async {
        // Create a Semaphore to limit concurrent tasks based on thread_count
        let semaphore = Arc::new(Semaphore::new(thread_count));

        // Spawn a tokio task for each file path
        // Each format_file call is wrapped as a tokio task and added to the task pool
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
        // AST parse errors cause immediate exit, other errors are printed and processing continues
        // Note: block_on will wait for this future, but we need to ensure all spawned tasks complete
        // block_on does NOT automatically wait for spawned tasks, so we must await them all

        let mut ast_parse_error = None;

        // Use futures::future::select_all to wait for tasks concurrently
        // This allows us to wait for any task to complete, not just sequentially
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
        // This ensures block_on waits for all spawned tasks before returning
        for handle in remaining_handles {
            // Await to ensure task cleanup (ignore results for aborted tasks)
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
/// Uses tokio::fs for async file I/O, and spawn_blocking for CPU-intensive parsing/formatting
async fn format_file_task(
    path: PathBuf,
    semaphore: Arc<Semaphore>,
    format_options: crate::FormatArgs,
) -> Result<(), String> {
    // Acquire permit to limit concurrency
    let _permit = semaphore
        .acquire()
        .await
        .map_err(|e| format!("Semaphore error: {}", e))?;

    // Use async file I/O for better performance in concurrent scenarios
    format_file_async(&path, format_options)
        .await
        .map_err(|err| format!("{}: {err}", path.display()))
}

/// Format a single file using async I/O
/// File I/O is async, CPU-intensive parsing/formatting runs in spawn_blocking
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
    // Use lossy UTF-8 conversion to handle non-UTF-8 content gracefully
    // Non-UTF-8 bytes will be replaced with the replacement character (�) without error
    let bytes = tokio::fs::read(&actual_path)
        .await
        .map_err(|e| format!("Failed to read file '{}': {}", actual_path.display(), e))?;

    let source_text = String::from_utf8_lossy(&bytes).into_owned();

    let source_type = SourceType::from_path(&actual_path)
        .map_err(|_| format!("Unsupported file type '{}'", actual_path.display()))?;

    // Skip empty files silently
    if source_text.is_empty() {
        return Ok(());
    }

    // Run CPU-intensive parsing and formatting in a blocking task
    let actual_path_clone = actual_path.clone();
    let formatted_code = tokio::task::spawn_blocking(move || {
        let allocator = Allocator::new();

        let ret = Parser::new(&allocator, &source_text, source_type)
            .with_options(get_parse_options())
            .parse();

        // If parsing fails, return error with special prefix to indicate AST parse error
        if !ret.errors.is_empty() {
            let mut error_msg = format!(
                "AST_PARSE_ERROR: Parser errors in '{}':\n",
                actual_path_clone.display()
            );
            for error in ret.errors {
                let error = error.with_source_code(source_text.clone());
                error_msg.push_str(&format!("{error:?}\n"));
            }
            return Err(error_msg);
        }

        // Build FormatOptions from command line arguments
        let mut option = FormatOptions {
            quote_properties: QuoteProperties::Preserve,
            ..Default::default()
        };

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
        // Note: experimental_sort_imports requires JSON parsing and is more complex
        // For now, we skip it. Users can configure it via config file if needed.

        let formatter = Formatter::new(&allocator, option);

        // Format the program
        // Note: If this panics with "begin <= end" error, it indicates a bug in the formatter
        // or an issue with the source code structure. The source_text reference should remain
        // valid throughout this call since it's a local variable.
        let formatted = formatter.format(&ret.program);
        let code = formatted
            .print()
            .map_err(|e| {
                format!(
                    "Failed to format file '{}': {}",
                    actual_path_clone.display(),
                    e
                )
            })?
            .into_code();

        Ok::<String, String>(code)
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
