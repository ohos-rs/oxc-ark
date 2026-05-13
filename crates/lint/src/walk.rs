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
                // Skip VCS metadata directories because they are not special cases for `.hidden(false)`.
                // <https://github.com/BurntSushi/ripgrep/issues/3099#issuecomment-3052460027>
                if entry.file_type().is_some_and(|ty| ty.is_dir())
                    && (entry.file_name() == ".git" || entry.file_name() == ".jj")
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

        let require_git = all_paths_have_vcs_boundary(paths);

        let inner = inner
            .ignore(false)
            .git_global(false)
            .git_ignore(true)
            .follow_links(true)
            .hidden(false)
            .require_git(require_git)
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

fn all_paths_have_vcs_boundary(paths: &[PathBuf]) -> bool {
    let cwd = std::env::current_dir().ok();
    let mut cache = FxHashMap::default();
    paths
        .iter()
        .all(|path| has_vcs_boundary(path, cwd.as_deref(), &mut cache))
}

fn has_vcs_boundary(path: &Path, cwd: Option<&Path>, cache: &mut FxHashMap<PathBuf, bool>) -> bool {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.map_or_else(|| path.to_path_buf(), |cwd| cwd.join(path))
    };

    let start = if path.is_file() {
        path.parent().unwrap_or(&path)
    } else {
        path.as_path()
    };

    if let Some(has_boundary) = cache.get(start) {
        return *has_boundary;
    }

    let has_boundary = start.ancestors().any(|dir| {
        cache
            .get(dir)
            .copied()
            .unwrap_or_else(|| dir.join(".git").exists() || dir.join(".jj").exists())
    });
    cache.insert(start.to_path_buf(), has_boundary);
    has_boundary
}
