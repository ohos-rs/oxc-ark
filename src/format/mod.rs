use std::{
    collections::HashSet,
    env,
    path::{Path, PathBuf},
    sync::Arc,
};

use futures::future;
use globwalk::GlobWalkerBuilder;
use oxc_allocator::Allocator;
use oxc_formatter::{Formatter, get_parse_options};
use oxc_parser::Parser;
use oxc_span::SourceType;
use tokio::sync::Semaphore;

pub fn format(args: crate::FormatArgs) -> Result<(), Box<dyn std::error::Error>> {
    let pattern = args.file;
    let thread_count = args.thread;

    if pattern.is_empty() {
        return Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            "Missing file pattern",
        )));
    }

    // 1) Exact file path (absolute or relative)
    let path = Path::new(&pattern);
    let mut files = if path.exists() && path.is_file() {
        vec![path.to_path_buf()]
    } else {
        // 2) Glob pattern (supports ** and brace sets)
        collect_matching_files(&pattern)?
    };

    // Deduplicate to ensure each file is formatted once across threads
    files = dedup_paths(files)?;

    if files.is_empty() {
        return Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("No files matched pattern '{pattern}'"),
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

            // Spawn format_file as a tokio task
            let handle = tokio::spawn(async move { format_file_task(path, semaphore).await });
            handles.push(handle);
        }

        // Wait for tasks to complete concurrently, exit immediately on first error
        // When a task fails, abort all remaining tasks and wait for them to finish
        // Note: block_on will wait for this future, but we need to ensure all spawned tasks complete
        // block_on does NOT automatically wait for spawned tasks, so we must await them all

        let mut first_error = None;

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
                    // Error occurred, abort all remaining tasks
                    first_error = Some(err);
                    // Abort all remaining tasks
                    for handle in remaining {
                        handle.abort();
                    }
                    remaining_handles = Vec::new();
                    break;
                }
                Err(e) => {
                    // Task panicked
                    first_error = Some(format!("Task panicked: {:?}", e));
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

        // Return error if any task failed
        if let Some(err) = first_error {
            return Err(
                Box::new(std::io::Error::new(std::io::ErrorKind::Other, err))
                    as Box<dyn std::error::Error>,
            );
        }

        Ok(())
    })
}

fn collect_matching_files(pattern: &str) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let mut seen = HashSet::new();
    let mut files = Vec::new();

    // Parse the pattern to determine root directory and glob pattern
    let (root, glob_pattern) = parse_glob_pattern(pattern)?;

    // Use globwalk to find matching files
    let walker = GlobWalkerBuilder::from_patterns(&root, &[&glob_pattern])
        .build()
        .map_err(|e| format!("Failed to build glob walker: {}", e))?;

    for entry in walker {
        match entry {
            Ok(entry) => {
                if entry.file_type().is_file() {
                    let path = entry.path().to_path_buf();
                    // Normalize and deduplicate immediately
                    // Use the original path from globwalk, as it should match the actual filesystem
                    let normalized = match normalize_path(&path) {
                        Ok(p) => {
                            // Verify normalized path still exists (case sensitivity check)
                            if p.exists() {
                                p
                            } else {
                                // If normalized path doesn't exist, use original path
                                // This handles case sensitivity issues on macOS
                                path
                            }
                        }
                        Err(_) => {
                            // If normalization fails, use original path
                            path
                        }
                    };
                    let key = normalized.to_string_lossy().into_owned();
                    if seen.insert(key) {
                        files.push(normalized);
                    }
                }
            }
            Err(e) => {
                // Log but continue for permission errors, etc.
                eprintln!("Warning: {}", e);
            }
        }
    }

    Ok(files)
}

fn parse_glob_pattern(pattern: &str) -> Result<(PathBuf, String), Box<dyn std::error::Error>> {
    // Find the first wildcard position
    let wildcard_pos = pattern
        .find(|c| matches!(c, '*' | '?' | '{' | '['))
        .unwrap_or_else(|| pattern.len());

    let prefix = &pattern[..wildcard_pos];
    let glob_part = &pattern[wildcard_pos..];

    if prefix.is_empty() {
        // Pattern starts with wildcard, use current directory as root
        Ok((std::env::current_dir()?, glob_part.to_string()))
    } else {
        // Extract the directory part before the first wildcard
        let prefix_path = Path::new(prefix);

        // Find the root directory (the deepest existing directory)
        let root = if prefix_path.is_absolute() {
            // For absolute paths, find the deepest existing directory
            let mut current = prefix_path.to_path_buf();
            while !current.exists() || !current.is_dir() {
                if let Some(parent) = current.parent() {
                    current = parent.to_path_buf();
                } else {
                    // Fallback to root directory
                    current = PathBuf::from("/");
                    break;
                }
            }
            current
        } else {
            // For relative paths, resolve against current directory
            let current_dir = std::env::current_dir()?;
            let full_path = current_dir.join(prefix_path);
            let mut current = full_path.clone();
            while !current.exists() || !current.is_dir() {
                if let Some(parent) = current.parent() {
                    if parent.starts_with(&current_dir) || parent == current_dir {
                        current = parent.to_path_buf();
                    } else {
                        current = current_dir.clone();
                        break;
                    }
                } else {
                    current = current_dir.clone();
                    break;
                }
            }
            current
        };

        // Calculate the glob pattern relative to root
        let glob_pattern = if prefix_path.is_absolute() {
            // For absolute paths, calculate relative path from root
            if root == prefix_path {
                glob_part.to_string()
            } else if let Ok(rel) = prefix_path.strip_prefix(&root) {
                if rel.as_os_str().is_empty() {
                    glob_part.to_string()
                } else {
                    format!("{}/{}", rel.to_string_lossy(), glob_part)
                }
            } else {
                // If can't strip prefix, use the full pattern from root
                if let Ok(rel) = prefix_path.strip_prefix("/") {
                    format!("{}/{}", rel.to_string_lossy(), glob_part)
                } else {
                    format!("{}/{}", prefix_path.to_string_lossy(), glob_part)
                }
            }
        } else {
            // For relative paths
            let current_dir = std::env::current_dir()?;
            let full_prefix = current_dir.join(prefix_path);
            if root == current_dir {
                // Root is current dir, use original pattern
                format!("{}/{}", prefix, glob_part)
            } else if let Ok(rel) = full_prefix.strip_prefix(&root) {
                if rel.as_os_str().is_empty() {
                    glob_part.to_string()
                } else {
                    format!("{}/{}", rel.to_string_lossy(), glob_part)
                }
            } else {
                // Fallback to original pattern
                format!("{}/{}", prefix, glob_part)
            }
        };

        Ok((root, glob_pattern))
    }
}

fn normalize_path(path: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    // Try to canonicalize first (resolves symlinks and makes absolute)
    // On macOS, canonicalize may change case, so we preserve the original path if canonicalize fails
    match path.canonicalize() {
        Ok(p) => {
            // Verify the canonicalized path still exists and is accessible
            if p.exists() {
                Ok(p)
            } else {
                // If canonicalized path doesn't exist, fall back to original
                if path.is_absolute() {
                    Ok(path.to_path_buf())
                } else {
                    Ok(env::current_dir()?.join(path))
                }
            }
        }
        Err(_) => {
            // If canonicalize fails (file doesn't exist or permission error),
            // try to make it absolute while preserving the original path
            if path.is_absolute() {
                Ok(path.to_path_buf())
            } else {
                // Make relative path absolute based on current directory
                Ok(env::current_dir()?.join(path))
            }
        }
    }
}

fn dedup_paths(paths: Vec<PathBuf>) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let mut seen = HashSet::new();
    let mut unique = Vec::new();

    for path in paths {
        let normalized = normalize_path(&path)?;
        let key = normalized.to_string_lossy().into_owned();
        if seen.insert(key) {
            unique.push(normalized);
        }
    }

    Ok(unique)
}

/// Format a single file as a tokio task
/// Uses tokio::fs for async file I/O, and spawn_blocking for CPU-intensive parsing/formatting
async fn format_file_task(path: PathBuf, semaphore: Arc<Semaphore>) -> Result<(), String> {
    // Acquire permit to limit concurrency
    let _permit = semaphore
        .acquire()
        .await
        .map_err(|e| format!("Semaphore error: {}", e))?;

    // Use async file I/O for better performance in concurrent scenarios
    format_file_async(&path)
        .await
        .map_err(|err| format!("{}: {err}", path.display()))
}

/// Format a single file using async I/O
/// File I/O is async, CPU-intensive parsing/formatting runs in spawn_blocking
async fn format_file_async(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    // Try to find the actual file path, handling case sensitivity issues on macOS
    let actual_path = if tokio::fs::metadata(path).await.is_ok() {
        // Path exists, use it directly
        path.to_path_buf()
    } else {
        // Path doesn't exist, try to canonicalize it (may resolve case issues)
        match path.canonicalize() {
            Ok(p) => {
                if p.exists() {
                    p
                } else {
                    return Err(Box::new(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        format!(
                            "File '{}' does not exist (canonicalized: '{}'). This may be a case sensitivity issue on macOS.",
                            path.display(),
                            p.display()
                        ),
                    )));
                }
            }
            Err(e) => {
                return Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!(
                        "File '{}' does not exist: {}. This may be a case sensitivity issue on macOS.",
                        path.display(),
                        e
                    ),
                )));
            }
        }
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

    // Ensure source_text is not empty
    if source_text.is_empty() {
        return Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("File '{}' is empty", path.display()),
        )));
    }

    // Run CPU-intensive parsing and formatting in a blocking task
    let actual_path_clone = actual_path.clone();
    let formatted_code = tokio::task::spawn_blocking(move || {
        let allocator = Allocator::new();

        let ret = Parser::new(&allocator, &source_text, source_type)
            .with_options(get_parse_options())
            .parse();

        // Check for parsing errors
        for error in ret.errors {
            let error = error.with_source_code(source_text.clone());
            println!("Parsing errors in file: {}", actual_path_clone.display());
            println!("{error:?}");
            return Err(format!(
                "Parsing errors in file: {}",
                actual_path_clone.display()
            ));
        }

        let formatter = Formatter::new(&allocator, Default::default());

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
