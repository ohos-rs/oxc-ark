use std::{
    collections::{HashMap, HashSet},
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
};

use fast_glob::glob_match;
use ignore::gitignore::{Gitignore, GitignoreBuilder};

use super::{FormatFileStrategy, should_ignore_file};

const NODE_MODULES_DIR: &str = "node_modules";
const OH_MODULES_DIR: &str = "oh_modules";

#[derive(Debug)]
pub struct FormatTargets {
    pub paths: Vec<PathBuf>,
    pub glob_patterns: Vec<String>,
    pub exclude_patterns: Vec<String>,
}

impl FormatTargets {
    #[must_use]
    pub fn new(cwd: &Path, patterns: &[String], excludes: &[String]) -> Self {
        let mut paths = Vec::new();
        let mut glob_patterns = Vec::new();
        let mut exclude_patterns = excludes.to_vec();

        for pattern in patterns {
            let pattern_path = Path::new(pattern);
            let pattern_str = pattern_path.to_string_lossy();

            if let Some(exclude) = pattern_str.strip_prefix('!') {
                exclude_patterns.push(exclude.to_string());
                continue;
            }

            let normalized = if let Some(stripped) = pattern_str.strip_prefix("./") {
                stripped.trim_start_matches('/')
            } else {
                &pattern_str
            };

            if is_glob_pattern(normalized, cwd) {
                glob_patterns.push(normalized.to_string());
                continue;
            }

            let path = Path::new(normalized);
            let full_path = if path.is_absolute() {
                path.to_path_buf()
            } else if normalized == "." {
                cwd.to_path_buf()
            } else {
                cwd.join(path)
            };
            paths.push(full_path);
        }

        Self {
            paths,
            glob_patterns,
            exclude_patterns,
        }
    }
}

fn is_glob_pattern(pattern: &str, cwd: &Path) -> bool {
    let has_glob_chars = pattern.contains('*')
        || pattern.contains('?')
        || pattern.contains('[')
        || pattern.contains('{');
    has_glob_chars && !cwd.join(pattern).exists()
}

struct GlobMatcher {
    cwd: PathBuf,
    patterns: Vec<String>,
}

impl GlobMatcher {
    fn new(cwd: PathBuf, patterns: Vec<String>) -> Self {
        let patterns = patterns
            .into_iter()
            .map(|pattern| {
                if pattern.contains('/') {
                    pattern
                } else {
                    format!("**/{pattern}")
                }
            })
            .collect();
        Self { cwd, patterns }
    }

    fn matches(&self, path: &Path) -> bool {
        let relative = path
            .strip_prefix(&self.cwd)
            .unwrap_or(path)
            .to_string_lossy();
        let absolute = path.to_string_lossy();
        self.patterns.iter().any(|pattern| {
            if Path::new(pattern).is_absolute() {
                glob_match(pattern, absolute.as_ref())
            } else {
                glob_match(pattern, relative.as_ref())
            }
        })
    }
}

struct CollectedFiles {
    files: Vec<PathBuf>,
    seen: HashSet<PathBuf>,
}

impl CollectedFiles {
    fn new() -> Self {
        Self {
            files: Vec::new(),
            seen: HashSet::new(),
        }
    }

    fn push_unique(&mut self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        if should_ignore_file(path) || FormatFileStrategy::try_from(path.to_path_buf()).is_err() {
            return Ok(());
        }

        let normalized = normalize_path(path)?;
        if self.seen.insert(normalized.clone()) {
            self.files.push(normalized);
        }
        Ok(())
    }

    fn into_sorted_files(mut self) -> Vec<PathBuf> {
        self.files.sort();
        self.files
    }
}

pub fn resolve_ignore_paths(cwd: &Path, ignore_paths: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    if !ignore_paths.is_empty() {
        let mut result = Vec::with_capacity(ignore_paths.len());
        for path in ignore_paths {
            let path = normalize_relative_path(cwd, path);
            if !path.exists() {
                return Err(format!("{}: File not found", path.display()));
            }
            result.push(path);
        }
        return Ok(result);
    }

    let prettierignore = cwd.join(".prettierignore");
    Ok(prettierignore
        .exists()
        .then_some(prettierignore)
        .into_iter()
        .collect())
}

pub fn build_global_ignore_matchers(
    cwd: &Path,
    exclude_patterns: &[String],
    ignore_paths: &[PathBuf],
) -> Result<Vec<Gitignore>, String> {
    let mut matchers = Vec::new();

    for ignore_path in ignore_paths {
        let (gitignore, err) = Gitignore::new(ignore_path);
        if let Some(err) = err {
            return Err(format!(
                "Failed to parse ignore file {}: {err}",
                ignore_path.display()
            ));
        }
        matchers.push(gitignore);
    }

    if !exclude_patterns.is_empty() {
        let mut builder = GitignoreBuilder::new(cwd);
        for pattern in exclude_patterns {
            if builder.add_line(None, pattern).is_err() {
                return Err(format!(
                    "Failed to add ignore pattern `{pattern}` from CLI exclude"
                ));
            }
        }
        matchers.push(
            builder
                .build()
                .map_err(|_| "Failed to build ignores".to_string())?,
        );
    }

    Ok(matchers)
}

pub fn is_global_ignored(
    matchers: &[Gitignore],
    path: &Path,
    is_dir: bool,
    check_ancestors: bool,
) -> bool {
    for matcher in matchers {
        let matched = if check_ancestors {
            if !path.starts_with(matcher.path()) {
                continue;
            }
            matcher.matched_path_or_any_parents(path, is_dir)
        } else {
            matcher.matched(path, is_dir)
        };
        if matched.is_ignore() && !matched.is_whitelist() {
            return true;
        }
    }
    false
}

pub fn collect_matching_files(
    cwd: &Path,
    targets: &FormatTargets,
    global_ignore_matchers: &[Gitignore],
    thread_count: usize,
    with_node_modules: bool,
) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let mut collected = CollectedFiles::new();

    let mut initial_targets: HashSet<PathBuf> = targets.paths.iter().cloned().collect();
    if !targets.glob_patterns.is_empty() {
        initial_targets.insert(cwd.to_path_buf());
    }
    if initial_targets.is_empty() && targets.glob_patterns.is_empty() {
        initial_targets.insert(cwd.to_path_buf());
    }

    let mut walk_targets = Vec::new();
    for path in initial_targets {
        let Ok(metadata) = path.metadata() else {
            continue;
        };
        let is_dir = metadata.is_dir();
        if is_global_ignored(global_ignore_matchers, &path, is_dir, true) {
            continue;
        }
        if is_dir {
            walk_targets.push(path);
        } else if metadata.is_file() {
            collected.push_unique(&path)?;
        }
    }

    if !walk_targets.is_empty() {
        let glob_matcher = (!targets.glob_patterns.is_empty())
            .then(|| GlobMatcher::new(cwd.to_path_buf(), targets.glob_patterns.clone()));
        collect_walked_files(
            cwd,
            &walk_targets,
            global_ignore_matchers,
            glob_matcher.as_ref(),
            thread_count,
            with_node_modules,
            &mut collected,
        )?;
    }

    Ok(collected.into_sorted_files())
}

fn collect_walked_files(
    cwd: &Path,
    walk_targets: &[PathBuf],
    global_ignore_matchers: &[Gitignore],
    glob_matcher: Option<&GlobMatcher>,
    thread_count: usize,
    with_node_modules: bool,
    collected: &mut CollectedFiles,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(first_path) = walk_targets.first() else {
        return Ok(());
    };

    let mut builder = ignore::WalkBuilder::new(first_path);
    for path in walk_targets.iter().skip(1) {
        builder.add(path);
    }

    let has_vcs_boundary = all_paths_have_vcs_boundary(walk_targets, cwd);
    configure_walk_builder(&mut builder, has_vcs_boundary)
        .follow_links(false)
        .threads(thread_count);

    for entry in builder.build() {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                eprintln!("Warning: {err}");
                continue;
            }
        };
        let Some(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            continue;
        }
        #[expect(clippy::filetype_is_file)]
        if !file_type.is_file() {
            continue;
        }

        let path = entry.path();
        if is_in_ignored_dir(path, with_node_modules)
            || is_global_ignored(global_ignore_matchers, path, false, true)
            || glob_matcher.is_some_and(|matcher| !matcher.matches(path))
        {
            continue;
        }

        collected.push_unique(path)?;
    }

    if with_node_modules {
        collect_package_module_files(
            walk_targets,
            global_ignore_matchers,
            glob_matcher,
            collected,
        )?;
    }

    Ok(())
}

fn collect_package_module_files(
    walk_targets: &[PathBuf],
    global_ignore_matchers: &[Gitignore],
    glob_matcher: Option<&GlobMatcher>,
    collected: &mut CollectedFiles,
) -> Result<(), Box<dyn std::error::Error>> {
    for target in walk_targets {
        find_package_module_dirs(target, global_ignore_matchers, glob_matcher, collected)?;
    }
    Ok(())
}

fn find_package_module_dirs(
    dir: &Path,
    global_ignore_matchers: &[Gitignore],
    glob_matcher: Option<&GlobMatcher>,
    collected: &mut CollectedFiles,
) -> Result<(), Box<dyn std::error::Error>> {
    if is_global_ignored(global_ignore_matchers, dir, true, true) {
        return Ok(());
    }

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if !file_type.is_dir() {
            continue;
        }

        let path = entry.path();
        if is_package_module_dir(&entry.file_name()) {
            collect_manual_dir(&path, global_ignore_matchers, glob_matcher, collected)?;
        } else if !is_ignored_dir(&entry.file_name(), true) {
            find_package_module_dirs(&path, global_ignore_matchers, glob_matcher, collected)?;
        }
    }

    Ok(())
}

fn collect_manual_dir(
    dir: &Path,
    global_ignore_matchers: &[Gitignore],
    glob_matcher: Option<&GlobMatcher>,
    collected: &mut CollectedFiles,
) -> Result<(), Box<dyn std::error::Error>> {
    if is_global_ignored(global_ignore_matchers, dir, true, true) {
        return Ok(());
    }

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let path = entry.path();
        if file_type.is_dir() {
            if !is_ignored_dir(&entry.file_name(), true) {
                collect_manual_dir(&path, global_ignore_matchers, glob_matcher, collected)?;
            }
            continue;
        }

        if !file_type.is_file()
            || is_global_ignored(global_ignore_matchers, &path, false, true)
            || glob_matcher.is_some_and(|matcher| !matcher.matches(&path))
        {
            continue;
        }

        collected.push_unique(&path)?;
    }

    Ok(())
}

fn configure_walk_builder(
    builder: &mut ignore::WalkBuilder,
    has_vcs_boundary: bool,
) -> &mut ignore::WalkBuilder {
    builder
        .hidden(false)
        .ignore(false)
        .git_global(false)
        .git_ignore(true)
        .parents(true)
        .git_exclude(true)
        .require_git(has_vcs_boundary)
}

fn all_paths_have_vcs_boundary(paths: &[PathBuf], cwd: &Path) -> bool {
    let mut cache = HashMap::default();
    paths
        .iter()
        .all(|path| has_vcs_boundary(path, cwd, &mut cache))
}

fn has_vcs_boundary(path: &Path, cwd: &Path, cache: &mut HashMap<PathBuf, bool>) -> bool {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    let start = if path.is_file() {
        path.parent().unwrap_or(&path)
    } else {
        path.as_path()
    };

    start.ancestors().any(|dir| {
        if let Some(&has) = cache.get(dir) {
            return has;
        }
        let has = dir.join(".git").exists() || dir.join(".jj").exists();
        cache.insert(dir.to_path_buf(), has);
        has
    })
}

fn is_in_ignored_dir(path: &Path, with_node_modules: bool) -> bool {
    path.ancestors()
        .filter_map(Path::file_name)
        .any(|name| is_ignored_dir(name, with_node_modules))
}

fn is_ignored_dir(dir_name: &OsStr, with_node_modules: bool) -> bool {
    dir_name == ".git"
        || dir_name == ".jj"
        || dir_name == ".sl"
        || dir_name == ".svn"
        || dir_name == ".hg"
        || (!with_node_modules && is_package_module_dir(dir_name))
}

fn is_package_module_dir(dir_name: &OsStr) -> bool {
    dir_name == OsStr::new(NODE_MODULES_DIR) || dir_name == OsStr::new(OH_MODULES_DIR)
}

fn normalize_relative_path(cwd: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    if let Ok(stripped) = path.strip_prefix("./") {
        cwd.join(stripped)
    } else {
        cwd.join(path)
    }
}

fn normalize_path(path: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    Ok(path
        .canonicalize()
        .or_else(|_| {
            if path.is_absolute() {
                Ok(path.to_path_buf())
            } else {
                std::env::current_dir().map(|cwd| cwd.join(path))
            }
        })
        .map_err(|e| std::io::Error::other(format!("Failed to normalize path: {e}")))?)
}

#[cfg(test)]
mod tests {
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
                "oxc-ark-format-discovery-{prefix}-{}-{}",
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
}
