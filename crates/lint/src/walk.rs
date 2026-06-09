// Portions of this file are derived from Oxc's oxlint implementation.
// Copyright (c) Oxc project contributors.
// Licensed under the MIT License. See https://github.com/oxc-project/oxc/blob/main/LICENSE.

#![allow(dead_code)]

use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    sync::Arc,
    sync::mpsc,
};

use ignore::{DirEntry, overrides::Override};
use oxc_linter::LINTABLE_EXTENSIONS;
use rustc_hash::FxHashMap;

use oxlint::cli::IgnoreOptions;

const GIT_DIR: &str = ".git";
const JJ_DIR: &str = ".jj";
const NODE_MODULES_DIR: &str = "node_modules";
const OH_MODULES_DIR: &str = "oh_modules";

#[derive(Debug, Clone)]
pub struct Extensions(pub Vec<&'static str>);

impl Default for Extensions {
    fn default() -> Self {
        Self(LINTABLE_EXTENSIONS.to_vec())
    }
}

pub struct Walk {
    inner: ignore::WalkParallel,
    /// The file extensions to include during the traversal.
    extensions: Extensions,
}

struct WalkBuilder {
    sender: mpsc::Sender<Vec<Arc<OsStr>>>,
    extensions: Extensions,
}

impl<'s> ignore::ParallelVisitorBuilder<'s> for WalkBuilder {
    fn build(&mut self) -> Box<dyn ignore::ParallelVisitor + 's> {
        Box::new(WalkCollector {
            paths: vec![],
            sender: self.sender.clone(),
            extensions: self.extensions.clone(),
        })
    }
}

struct WalkCollector {
    paths: Vec<Arc<OsStr>>,
    sender: mpsc::Sender<Vec<Arc<OsStr>>>,
    extensions: Extensions,
}

impl Drop for WalkCollector {
    fn drop(&mut self) {
        let paths = std::mem::take(&mut self.paths);
        self.sender.send(paths).unwrap();
    }
}

impl ignore::ParallelVisitor for WalkCollector {
    fn visit(&mut self, entry: Result<ignore::DirEntry, ignore::Error>) -> ignore::WalkState {
        match entry {
            Ok(entry) => {
                // Skip VCS metadata and dependency directories before file collection.
                // VCS metadata directories are not special-cased for `.hidden(false)`.
                // <https://github.com/BurntSushi/ripgrep/issues/3099#issuecomment-3052460027>
                if entry.file_type().is_some_and(|ty| ty.is_dir())
                    && is_skipped_dir(entry.file_name())
                {
                    return ignore::WalkState::Skip;
                }
                if Walk::is_wanted_entry(&entry, &self.extensions) {
                    self.paths.push(entry.path().as_os_str().into());
                }
                ignore::WalkState::Continue
            }
            Err(_err) => ignore::WalkState::Skip,
        }
    }
}
impl Walk {
    /// Will not canonicalize paths.
    /// # Panics
    pub fn new(
        paths: &[PathBuf],
        cwd: &Path,
        options: &IgnoreOptions,
        override_builder: Option<Override>,
    ) -> Self {
        assert!(
            !paths.is_empty(),
            "At least one path must be provided to Walk::new"
        );

        let mut inner = ignore::WalkBuilder::new(
            paths
                .iter()
                .next()
                .expect("Expected paths parameter to Walk::new() to contain at least one path."),
        );

        if let Some(paths) = paths.get(1..) {
            for path in paths {
                inner.add(path);
            }
        }

        if !options.no_ignore {
            inner.add_custom_ignore_filename(&options.ignore_path);

            if let Some(override_builder) = override_builder {
                inner.overrides(override_builder);
            }
        }

        let has_vcs_boundary = all_paths_have_vcs_boundary(paths, cwd);

        let inner = configure_walk_builder(&mut inner, has_vcs_boundary)
            .follow_links(true)
            .threads(rayon::current_num_threads())
            .build_parallel();
        Self {
            inner,
            extensions: Extensions::default(),
        }
    }

    pub fn paths(self) -> Vec<Arc<OsStr>> {
        let (sender, receiver) = mpsc::channel::<Vec<Arc<OsStr>>>();
        let mut builder = WalkBuilder {
            sender,
            extensions: self.extensions,
        };
        self.inner.visit(&mut builder);
        drop(builder);
        receiver.into_iter().flatten().collect()
    }

    #[cfg_attr(not(test), expect(dead_code))]
    pub fn with_extensions(mut self, extensions: Extensions) -> Self {
        self.extensions = extensions;
        self
    }

    fn is_wanted_entry(dir_entry: &DirEntry, extensions: &Extensions) -> bool {
        let Some(file_type) = dir_entry.file_type() else {
            return false;
        };
        if file_type.is_dir() {
            return false;
        }
        let Some(file_name) = dir_entry.path().file_name() else {
            return false;
        };
        let file_name = file_name.to_string_lossy();
        let file_name = file_name.as_ref();
        if [".min.", "-min.", "_min."]
            .iter()
            .any(|e| file_name.contains(e))
        {
            return false;
        }
        let Some(extension) = dir_entry.path().extension() else {
            return false;
        };
        let extension = extension.to_string_lossy();
        extensions.0.contains(&extension.as_ref())
    }
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
    let mut cache = FxHashMap::default();
    paths
        .iter()
        .all(|path| has_vcs_boundary(path, cwd, &mut cache))
}

fn has_vcs_boundary(path: &Path, cwd: &Path, cache: &mut FxHashMap<PathBuf, bool>) -> bool {
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

fn is_skipped_dir(dir_name: &OsStr) -> bool {
    dir_name == OsStr::new(GIT_DIR)
        || dir_name == OsStr::new(JJ_DIR)
        || dir_name == OsStr::new(NODE_MODULES_DIR)
        || dir_name == OsStr::new(OH_MODULES_DIR)
}

#[cfg(test)]
mod tests {
    use std::{
        ffi::OsString,
        fs,
        path::{Path, PathBuf},
        process,
        sync::atomic::{AtomicU64, Ordering},
    };

    use oxlint::cli::IgnoreOptions;

    use super::{Extensions, Walk};

    static NEXT_TEST_DIR_ID: AtomicU64 = AtomicU64::new(0);

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(prefix: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "oxc-ark-lint-walk-{prefix}-{}-{}",
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

    fn default_ignore_options() -> IgnoreOptions {
        IgnoreOptions {
            ignore_path: OsString::from(".eslintignore"),
            ignore_pattern: vec![],
            no_ignore: false,
        }
    }

    #[test]
    fn walk_skips_package_modules_by_default() {
        let dir = TestDir::new("package-modules");
        fs::create_dir_all(dir.join("src")).expect("src dir should be created");
        fs::create_dir_all(dir.join("node_modules/pkg"))
            .expect("node_modules dir should be created");
        fs::create_dir_all(dir.join("oh_modules/pkg")).expect("oh_modules dir should be created");
        fs::write(dir.join("src/index.ts"), "const a = 1;\n").expect("src file should be written");
        fs::write(dir.join("node_modules/pkg/index.ts"), "const b = 1;\n")
            .expect("node_modules file should be written");
        fs::write(dir.join("oh_modules/pkg/index.ts"), "const c = 1;\n")
            .expect("oh_modules file should be written");

        let mut paths = Walk::new(
            &[dir.path().to_path_buf()],
            dir.path(),
            &default_ignore_options(),
            None,
        )
        .with_extensions(Extensions(vec!["ts"]))
        .paths()
        .into_iter()
        .map(|path| {
            Path::new(path.as_ref())
                .strip_prefix(dir.path())
                .expect("path should be under temp root")
                .to_string_lossy()
                .to_string()
        })
        .collect::<Vec<_>>();
        paths.sort();

        assert_eq!(paths, vec!["src/index.ts"]);
    }
}
