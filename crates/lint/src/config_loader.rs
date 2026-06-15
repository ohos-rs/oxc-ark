// Portions of this file are derived from Oxc's oxlint implementation.
// Copyright (c) Oxc project contributors.
// Licensed under the MIT License. See https://github.com/oxc-project/oxc/blob/main/LICENSE.

#![allow(dead_code)]

use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
};

use ignore::DirEntry;

use oxc_config_discovery::{
    ConfigConflict, ConfigDiscovery, ConfigFileNames, DiscoveredConfigFile, is_js_config_path,
};
use oxc_diagnostics::OxcDiagnostic;
use oxc_linter::{
    Config, ConfigStoreBuilder, ExternalLinter, ExternalPluginStore, LintFilter, Oxlintrc,
};
use rustc_hash::{FxBuildHasher, FxHashMap, FxHashSet};

use crate::{
    ArktsLintConfig, DEFAULT_JSONC_OXLINTRC_NAME, DEFAULT_OXLINTRC_NAME, DEFAULT_TS_OXLINTRC_NAME,
    load_oxlintrc_and_arkts_from_file,
};

const GIT_DIR: &str = ".git";
const NODE_MODULES_DIR: &str = "node_modules";
const OH_MODULES_DIR: &str = "oh_modules";

pub struct JsConfigResult {
    pub config: Option<Oxlintrc>,
}

const OXLINT_CONFIG_FILE_NAMES: ConfigFileNames = ConfigFileNames {
    json: DEFAULT_OXLINTRC_NAME,
    jsonc: DEFAULT_JSONC_OXLINTRC_NAME,
    js: DEFAULT_TS_OXLINTRC_NAME,
    vite: "vite.config.ts",
};

fn config_discovery() -> ConfigDiscovery {
    ConfigDiscovery::new(OXLINT_CONFIG_FILE_NAMES, false)
}

pub fn config_file_names() -> Vec<&'static str> {
    config_discovery().config_file_names()
}

/// Discover config files by walking UP from each file's directory to ancestors.
///
/// Used by CLI where we have specific files to lint and need to find configs
/// that apply to them.
///
/// Example: For files `/project/src/foo.js` and `/project/src/bar/baz.js`:
/// - Checks `/project/src/bar/`, `/project/src/`, `/project/`, `/`
/// - Returns paths to matching config files found
///
/// In Vite+ mode, only `vite.config.ts` is discovered.
pub fn discover_configs_in_ancestors<P: AsRef<Path>>(
    files: &[P],
    base_config_path: &Path,
) -> impl IntoIterator<Item = DiscoveredConfigFile> {
    let mut config_paths = FxHashSet::<DiscoveredConfigFile>::default();
    let mut visited_dirs = FxHashSet::default();

    for file in files {
        let path = file.as_ref();
        let mut base_config_found = false;
        // Start from the file's parent directory and walk up the tree
        let mut current = path.parent();
        while let Some(dir) = current {
            if base_config_found {
                // Stop if we've reached the base config file (e.g., root oxlintrc)
                // to avoid duplicate loading and filling nested config with configs outside from the root config.
                break;
            }
            // Stop if we've already checked this directory (and its ancestors)
            let inserted = visited_dirs.insert(dir.to_path_buf());
            if !inserted {
                break;
            }
            for config in find_configs_in_directory(dir) {
                if config.path() == base_config_path {
                    base_config_found = true;
                    break;
                }
                config_paths.insert(config);
            }
            current = dir.parent();
        }
    }

    config_paths
}

/// Discover config files by walking DOWN from a root directory.
/// Will skip the base config file (e.g., root oxlintrc) to avoid duplicate loading.
/// In Vite+ mode, only `vite.config.ts` is discovered.
///
/// Used by LSP where we have a workspace root and need to discover all configs
/// upfront for file watching and diagnostics.
pub fn discover_configs_in_tree(
    root: &Path,
    base_config_path: &Path,
) -> impl IntoIterator<Item = DiscoveredConfigFile> {
    let walker = ignore::WalkBuilder::new(root)
        .hidden(false) // don't skip hidden files
        .parents(false) // disable gitignore from parent dirs
        .ignore(false) // disable .ignore files
        .git_global(false) // disable global gitignore
        .follow_links(true)
        .build_parallel();

    let (sender, receiver) = mpsc::channel::<Vec<DiscoveredConfigFile>>();
    let mut builder = ConfigWalkBuilder {
        sender,
        base_config_path: base_config_path.to_path_buf(),
    };
    walker.visit(&mut builder);
    drop(builder);

    receiver.into_iter().flatten()
}

/// Check if a directory contains an oxlint config file.
fn find_configs_in_directory(dir: &Path) -> Vec<DiscoveredConfigFile> {
    config_discovery().find_configs_in_directory(dir)
}

// Helper types for parallel directory walking
struct ConfigWalkBuilder {
    sender: mpsc::Sender<Vec<DiscoveredConfigFile>>,
    base_config_path: PathBuf,
}

impl<'s> ignore::ParallelVisitorBuilder<'s> for ConfigWalkBuilder {
    fn build(&mut self) -> Box<dyn ignore::ParallelVisitor + 's> {
        Box::new(ConfigWalkCollector {
            configs: vec![],
            sender: self.sender.clone(),
            base_config_path: self.base_config_path.clone(),
        })
    }
}

struct ConfigWalkCollector {
    configs: Vec<DiscoveredConfigFile>,
    sender: mpsc::Sender<Vec<DiscoveredConfigFile>>,
    base_config_path: PathBuf,
}

impl Drop for ConfigWalkCollector {
    fn drop(&mut self) {
        let configs = std::mem::take(&mut self.configs);
        self.sender.send(configs).unwrap();
    }
}

impl ignore::ParallelVisitor for ConfigWalkCollector {
    fn visit(&mut self, entry: Result<DirEntry, ignore::Error>) -> ignore::WalkState {
        match entry {
            Ok(entry) => {
                // Skip dependency and VCS metadata directories; they are not part of the
                // lintable project tree for config discovery.
                if entry.file_type().is_some_and(|ft| ft.is_dir())
                    && is_skipped_config_dir(entry.file_name())
                {
                    return ignore::WalkState::Skip;
                }
                if let Some(config) = to_discovered_config(&entry, &self.base_config_path) {
                    self.configs.push(config);
                }
                ignore::WalkState::Continue
            }
            Err(_) => ignore::WalkState::Skip,
        }
    }
}

fn is_skipped_config_dir(dir_name: &OsStr) -> bool {
    dir_name == OsStr::new(GIT_DIR)
        || dir_name == OsStr::new(NODE_MODULES_DIR)
        || dir_name == OsStr::new(OH_MODULES_DIR)
}

fn to_discovered_config(entry: &DirEntry, base_config_path: &Path) -> Option<DiscoveredConfigFile> {
    let file_type = entry.file_type()?;
    if file_type.is_dir() {
        return None;
    }
    if entry.path() == base_config_path {
        // Skip the base config file (e.g., root oxlintrc) to avoid duplicate loading
        return None;
    }
    config_discovery().discover_config_file(entry.path())
}

pub struct LoadedConfig {
    /// The directory this config applies to
    pub dir: PathBuf,
    /// The built configuration
    pub config: Config,
    /// Ignore patterns from this config
    pub ignore_patterns: Vec<String>,
    /// Paths from extends directives
    pub extended_paths: Vec<PathBuf>,
}

/// Errors that can occur when loading configs
#[derive(Debug)]
pub enum ConfigLoadError {
    /// Failed to parse the config file
    Parse {
        path: PathBuf,
        error: OxcDiagnostic,
    },
    /// Failed to build the ConfigStore
    Build {
        path: PathBuf,
        error: String,
    },

    JsConfigFileFoundButJsRuntimeNotAvailable,

    Diagnostic(OxcDiagnostic),
}

impl ConfigLoadError {
    /// Get the path of the config file that failed
    pub fn path(&self) -> Option<&Path> {
        match self {
            ConfigLoadError::Parse { path, .. } | ConfigLoadError::Build { path, .. } => Some(path),
            _ => None,
        }
    }
}

/// High-level errors that can occur when loading CLI configurations.
///
/// This groups together failures related to the root configuration file
/// and to any nested configuration files discovered during loading.
pub enum CliConfigLoadError {
    /// An error that occurred while loading or parsing the root configuration.
    RootConfig(OxcDiagnostic),
    /// One or more errors that occurred while loading nested configuration files.
    NestedConfigs(Vec<ConfigLoadError>),
}

/// Collection of the root configuration and all successfully loaded nested configs.
///
/// Returned by [`ConfigLoader::load_root_and_nested`].
pub struct LoadedConfigs {
    /// The root `oxlintrc` configuration used as the base for all linting.
    pub root: Oxlintrc,
    /// Mapping from directory paths to the effective [`Config`] for that directory.
    pub nested: FxHashMap<PathBuf, Config>,
    /// Ignore patterns from nested configs, paired with the directory they apply to.
    pub nested_ignore_patterns: Vec<(Vec<String>, PathBuf)>,
}

pub struct ConfigLoader<'a> {
    external_linter: Option<&'a ExternalLinter>,
    external_plugin_store: &'a mut ExternalPluginStore,
    filters: &'a [LintFilter],
    workspace_uri: Option<&'a str>,
}

impl<'a> ConfigLoader<'a> {
    /// Create a new ConfigLoader
    ///
    /// # Arguments
    /// * `external_linter` - Optional external linter for plugin support
    /// * `external_plugin_store` - Store for external plugins
    /// * `filters` - Lint filters to apply to configs
    /// * `workspace_uri` - Workspace URI  - only `Some` in LSP, `None` in CLI
    pub fn new(
        external_linter: Option<&'a ExternalLinter>,
        external_plugin_store: &'a mut ExternalPluginStore,
        filters: &'a [LintFilter],
        workspace_uri: Option<&'a str>,
    ) -> Self {
        Self {
            external_linter,
            external_plugin_store,
            filters,
            workspace_uri,
        }
    }

    /// Load a single config from a file path
    fn load(path: &Path) -> Result<Oxlintrc, ConfigLoadError> {
        load_oxlintrc_and_arkts_from_file(path)
            .map(|(config, _)| config)
            .map_err(|error| ConfigLoadError::Parse {
                path: path.to_path_buf(),
                error,
            })
    }

    pub fn load_js_configs(
        &self,
        paths: &[PathBuf],
    ) -> Result<Vec<JsConfigResult>, Vec<ConfigLoadError>> {
        if paths.is_empty() {
            return Ok(Vec::new());
        }

        Err(vec![
            ConfigLoadError::JsConfigFileFoundButJsRuntimeNotAvailable,
        ])
    }

    /// Load multiple configs, returning successes and errors separately
    ///
    /// This allows callers to decide how to handle errors (fail fast vs continue)
    fn load_many(
        &mut self,
        paths: impl IntoIterator<Item = DiscoveredConfigFile>,
        root_config_dir: Option<&Path>,
    ) -> (Vec<LoadedConfig>, Vec<ConfigLoadError>) {
        let mut configs = Vec::new();
        let mut errors = Vec::new();

        let mut by_dir = FxHashMap::<PathBuf, Vec<DiscoveredConfigFile>>::default();

        for config in paths {
            let Some(dir) = config.path().parent().map(Path::to_path_buf) else {
                continue;
            };

            by_dir.entry(dir).or_default().push(config);
        }

        let mut js_configs = Vec::new();

        for (dir, config_files) in by_dir {
            if config_files.len() > 1 {
                errors.push(ConfigLoadError::Diagnostic(
                    ConfigConflict::new(dir.clone(), config_files).into(),
                ));
                continue;
            }

            match config_files.into_iter().next() {
                Some(DiscoveredConfigFile::Json(path) | DiscoveredConfigFile::Jsonc(path)) => {
                    match Self::load(path.as_path()) {
                        Ok(config) => configs.push(config),
                        Err(e) => errors.push(e),
                    }
                }
                Some(DiscoveredConfigFile::Js(path) | DiscoveredConfigFile::Vite(path)) => {
                    js_configs.push(path);
                }
                None => {
                    debug_assert!(
                        false,
                        "Expected at least one config file for directory {}",
                        dir.display()
                    );
                }
            }
        }

        match self.load_js_configs(&js_configs) {
            Ok(loaded_js_configs) => {
                configs.extend(loaded_js_configs.into_iter().filter_map(|c| c.config));
            }
            Err(mut js_errors) => {
                errors.append(&mut js_errors);
            }
        }

        let mut built_configs = Vec::new();

        for config in configs {
            let path = config.path.clone();
            let dir = path.parent().unwrap().to_path_buf();
            let ignore_patterns = config.ignore_patterns.clone();
            let is_root_config = root_config_dir
                .and_then(|root| path.parent().map(|parent| parent == root))
                .unwrap_or(false);

            if !is_root_config {
                let options = &config.options;
                if options.type_aware.is_some() {
                    errors.push(ConfigLoadError::Diagnostic(
                        nested_type_aware_not_supported(&path),
                    ));
                    continue;
                }
                if options.type_check.is_some() {
                    errors.push(ConfigLoadError::Diagnostic(
                        nested_type_check_not_supported(&path),
                    ));
                    continue;
                }
                if options.deny_warnings.is_some() {
                    errors.push(ConfigLoadError::Diagnostic(
                        nested_deny_warnings_not_supported(&path),
                    ));
                    continue;
                }
                if options.max_warnings.is_some() {
                    errors.push(ConfigLoadError::Diagnostic(
                        nested_max_warnings_not_supported(&path),
                    ));
                    continue;
                }
                if options.report_unused_disable_directives.is_some() {
                    errors.push(ConfigLoadError::Diagnostic(
                        nested_report_unused_disable_directives_not_supported(&path),
                    ));
                    continue;
                }
                if options.respect_eslint_disable_directives.is_some() {
                    errors.push(ConfigLoadError::Diagnostic(
                        nested_respect_eslint_disable_directives_not_supported(&path),
                    ));
                    continue;
                }
            }

            let builder = match ConfigStoreBuilder::from_oxlintrc(
                false,
                config,
                self.external_linter,
                self.external_plugin_store,
                self.workspace_uri,
            ) {
                Ok(builder) => builder,
                Err(e) => {
                    errors.push(ConfigLoadError::Build {
                        path,
                        error: e.to_string(),
                    });
                    continue;
                }
            };

            let extended_paths = builder.extended_paths.clone();

            match builder
                .with_filters(self.filters)
                .build(self.external_plugin_store)
                .map_err(|e| ConfigLoadError::Build {
                    path: path.clone(),
                    error: e.to_string(),
                }) {
                Ok(config) => built_configs.push(LoadedConfig {
                    dir,
                    config,
                    ignore_patterns,
                    extended_paths,
                }),
                Err(e) => errors.push(e),
            }
        }

        (built_configs, errors)
    }

    pub(crate) fn load_discovered_with_root_dir(
        &mut self,
        root_dir: &Path,
        configs: impl IntoIterator<Item = DiscoveredConfigFile>,
    ) -> (Vec<LoadedConfig>, Vec<ConfigLoadError>) {
        self.load_many(configs, Some(root_dir))
    }

    /// Try to load config from a specific directory.
    ///
    /// In Vite+ mode (`VP_VERSION` set): only checks for `vite.config.ts`.
    /// Otherwise: checks for `.oxlintrc.json`, `.oxlintrc.jsonc`, and `oxlint.config.ts`.
    ///
    /// Returns `Ok(Some(config))` if found, `Ok(None)` if not found, or `Err` on error.
    fn try_load_config_from_dir(&self, dir: &Path) -> Result<Option<Oxlintrc>, OxcDiagnostic> {
        let config_file = config_discovery()
            .find_unique_config_in_directory(dir)
            .map_err(OxcDiagnostic::from)?;

        match config_file {
            Some(DiscoveredConfigFile::Json(path) | DiscoveredConfigFile::Jsonc(path)) => {
                load_oxlintrc_and_arkts_from_file(&path).map(|(config, _)| Some(config))
            }
            Some(DiscoveredConfigFile::Js(path)) => {
                let config = self.load_root_js_config(&path)?;
                debug_assert!(
                    config.is_some(),
                    "oxlint.config.ts should always return a config"
                );
                Ok(config)
            }
            Some(DiscoveredConfigFile::Vite(path)) => self.load_root_js_config(&path),
            None => Ok(None),
        }
    }

    fn try_load_config_and_arkts_from_dir(
        &self,
        dir: &Path,
    ) -> Result<Option<(Oxlintrc, ArktsLintConfig)>, OxcDiagnostic> {
        let config_file = config_discovery()
            .find_unique_config_in_directory(dir)
            .map_err(OxcDiagnostic::from)?;

        match config_file {
            Some(DiscoveredConfigFile::Json(path) | DiscoveredConfigFile::Jsonc(path)) => {
                load_oxlintrc_and_arkts_from_file(&path).map(Some)
            }
            Some(DiscoveredConfigFile::Js(path)) => {
                let config = self.load_root_js_config(&path)?;
                debug_assert!(
                    config.is_some(),
                    "oxlint.config.ts should always return a config"
                );
                Ok(config.map(|config| (config, ArktsLintConfig::default())))
            }
            Some(DiscoveredConfigFile::Vite(path)) => Ok(self
                .load_root_js_config(&path)?
                .map(|config| (config, ArktsLintConfig::default()))),
            None => Ok(None),
        }
    }

    pub(crate) fn load_root_config(
        &self,
        cwd: &Path,
        config_path: Option<&PathBuf>,
    ) -> Result<Oxlintrc, OxcDiagnostic> {
        if let Some(config_path) = config_path {
            return self.load_explicit_config(cwd, config_path);
        }

        match self.try_load_config_from_dir(cwd)? {
            Some(config) => Ok(config),
            None => Ok(Oxlintrc::default()),
        }
    }

    /// Load root config by searching up parent directories.
    ///
    /// This is used by the LSP when a workspace folder is nested (e.g., `apps/app1`).
    /// It searches from the current directory up to parent directories to find a config file.
    ///
    /// # Arguments
    /// * `cwd` - Current working directory (workspace root for LSP)
    /// * `config_path` - Optional explicit path to the root config file
    ///
    /// # Returns
    /// The first config found when searching up the directory tree, or default if none found.
    pub(crate) fn load_root_config_with_ancestor_search(
        &self,
        cwd: &Path,
        config_path: Option<&PathBuf>,
    ) -> Result<Oxlintrc, OxcDiagnostic> {
        // If an explicit config path is provided, use it directly
        if let Some(config_path) = config_path {
            return self.load_explicit_config(cwd, config_path);
        }

        // Search up the directory tree for a config file
        let mut current = Some(cwd);
        while let Some(dir) = current {
            if let Some(config) = self.try_load_config_from_dir(dir)? {
                return Ok(config);
            }
            // Move to parent directory
            current = dir.parent();
        }

        // No config found in any ancestor directory
        Ok(Oxlintrc::default())
    }

    pub(crate) fn load_root_config_with_arkts_ancestor_search(
        &self,
        cwd: &Path,
        config_path: Option<&PathBuf>,
    ) -> Result<(Oxlintrc, ArktsLintConfig), OxcDiagnostic> {
        if let Some(config_path) = config_path {
            return self.load_explicit_config_with_arkts(cwd, config_path);
        }

        let mut current = Some(cwd);
        while let Some(dir) = current {
            if let Some(config) = self.try_load_config_and_arkts_from_dir(dir)? {
                return Ok(config);
            }
            current = dir.parent();
        }

        Ok((Oxlintrc::default(), ArktsLintConfig::default()))
    }

    /// Load an explicitly specified config file (via `--config`).
    /// For JS/TS configs, `None` from JS side (e.g., vite.config.ts without `.lint`) is an error.
    fn load_explicit_config(
        &self,
        cwd: &Path,
        config_path: &Path,
    ) -> Result<Oxlintrc, OxcDiagnostic> {
        let full_path = cwd.join(config_path);
        if is_js_config_path(&full_path) {
            return self.load_root_js_config(&full_path)?.ok_or_else(|| {
                OxcDiagnostic::error(format!(
                    "Expected a `lint` field in the default export of {}",
                    full_path.display()
                ))
            });
        }
        load_oxlintrc_and_arkts_from_file(&full_path).map(|(config, _)| config)
    }

    fn load_explicit_config_with_arkts(
        &self,
        cwd: &Path,
        config_path: &Path,
    ) -> Result<(Oxlintrc, ArktsLintConfig), OxcDiagnostic> {
        let full_path = cwd.join(config_path);
        if is_js_config_path(&full_path) {
            return self
                .load_root_js_config(&full_path)?
                .map(|config| (config, ArktsLintConfig::default()))
                .ok_or_else(|| {
                    OxcDiagnostic::error(format!(
                        "Expected a `lint` field in the default export of {}",
                        full_path.display()
                    ))
                });
        }
        load_oxlintrc_and_arkts_from_file(&full_path)
    }

    /// Load a single JS/TS config file. Returns `Ok(None)` when JS side signals "skip"
    /// (e.g., vite.config.ts without `.lint` field).
    fn load_root_js_config(&self, path: &Path) -> Result<Option<Oxlintrc>, OxcDiagnostic> {
        match self.load_js_configs(&[path.to_path_buf()]) {
            Ok(mut results) => Ok(results.pop().and_then(|r| r.config)),
            Err(errors) => {
                if let Some(first) = errors.into_iter().next() {
                    match first {
                        ConfigLoadError::JsConfigFileFoundButJsRuntimeNotAvailable => {
                            Err(js_config_not_supported_diagnostic(path))
                        }
                        ConfigLoadError::Diagnostic(diag) => Err(diag),
                        // `load_js_configs` only returns the two variants above, but keep this
                        // resilient if that changes.
                        ConfigLoadError::Parse { error, .. } => Err(error),
                        ConfigLoadError::Build { error, .. } => Err(OxcDiagnostic::error(error)),
                    }
                } else {
                    Err(OxcDiagnostic::error(
                        "Failed to load JavaScript/TypeScript config.",
                    ))
                }
            }
        }
    }

    /// Load the root configuration and optionally discover and load nested configs.
    ///
    /// This is the main entry point for CLI config loading. It first loads the root
    /// `oxlintrc` configuration, then optionally discovers and loads nested configs
    /// by walking up from each file path's directory.
    ///
    /// # Arguments
    /// * `cwd` - Current working directory for resolving relative paths
    /// * `config_path` - Optional explicit path to the root config file
    /// * `paths` - File paths to lint (used for discovering nested configs)
    /// * `search_for_nested_configs` - Whether to discover nested configs in ancestor directories
    ///
    /// # Errors
    /// Returns [`CliConfigLoadError::RootConfig`] if the root config fails to load,
    /// or [`CliConfigLoadError::NestedConfigs`] if any nested config fails to load.
    pub fn load_root_and_nested(
        &mut self,
        cwd: &Path,
        config_path: Option<&PathBuf>,
        paths: &[Arc<OsStr>],
        search_for_nested_configs: bool,
    ) -> Result<LoadedConfigs, CliConfigLoadError> {
        let oxlintrc = match self.load_root_config(cwd, config_path) {
            Ok(config) => config,
            Err(err) => return Err(CliConfigLoadError::RootConfig(err)),
        };

        if !search_for_nested_configs {
            return Ok(LoadedConfigs {
                root: oxlintrc,
                nested: FxHashMap::default(),
                nested_ignore_patterns: vec![],
            });
        }

        // Discover config files by walking up from each file's directory
        let config_paths: Vec<_> = paths
            .iter()
            .map(|p| Path::new(p.as_ref()).to_path_buf())
            .collect();
        let discovered_configs = discover_configs_in_ancestors(&config_paths, &oxlintrc.path);

        let (configs, errors) = self.load_many(discovered_configs, Some(cwd));

        // Fail if any config failed (CLI requires all configs to be valid)
        if !errors.is_empty() {
            return Err(CliConfigLoadError::NestedConfigs(errors));
        }

        // Convert loaded configs to nested config format
        let mut nested_ignore_patterns = Vec::with_capacity(configs.len());
        let nested_configs = build_nested_configs(configs, &mut nested_ignore_patterns, None);

        Ok(LoadedConfigs {
            root: oxlintrc,
            nested: nested_configs,
            nested_ignore_patterns,
        })
    }
}

/// Build a map of directory paths to their effective configurations.
///
/// Processes a list of loaded configs and organizes them into a hashmap keyed by
/// directory path. Also collects ignore patterns and optionally tracks extended paths.
///
/// # Arguments
/// * `configs` - Successfully loaded configurations to process
/// * `nested_ignore_patterns` - Output: populated with (ignore_patterns, directory) tuples
/// * `extended_paths` - Optional set to collect paths from `extends` directives.
///   Pass `Some` when tracking extended configs for file watching (LSP), `None` otherwise (CLI).
pub fn build_nested_configs(
    configs: Vec<LoadedConfig>,
    nested_ignore_patterns: &mut Vec<(Vec<String>, PathBuf)>,
    mut extended_paths: Option<&mut FxHashSet<PathBuf>>,
) -> FxHashMap<PathBuf, Config> {
    let mut nested_configs =
        FxHashMap::<PathBuf, Config>::with_capacity_and_hasher(configs.len(), FxBuildHasher);

    for loaded in configs {
        nested_ignore_patterns.push((loaded.ignore_patterns, loaded.dir.clone()));
        if let Some(extended_paths) = extended_paths.as_deref_mut() {
            extended_paths.extend(loaded.extended_paths);
        }
        nested_configs.insert(loaded.dir, loaded.config);
    }

    nested_configs
}

fn js_config_not_supported_diagnostic(path: &Path) -> OxcDiagnostic {
    OxcDiagnostic::error(format!(
        "JavaScript/TypeScript config file ({}) found but JS runtime not available.",
        path.display()
    ))
    .with_help("Run oxlint via the npm package, or use JSON config files (.oxlintrc.json or .oxlintrc.jsonc).")
}

fn nested_type_aware_not_supported(path: &Path) -> OxcDiagnostic {
    OxcDiagnostic::error(format!(
        "The `options.typeAware` option is only supported in the root config, but it was found in {}.",
        path.display()
    ))
    .with_help("Move `options.typeAware` to the root configuration file.")
}

fn nested_type_check_not_supported(path: &Path) -> OxcDiagnostic {
    OxcDiagnostic::error(format!(
        "The `options.typeCheck` option is only supported in the root config, but it was found in {}.",
        path.display()
    ))
    .with_help("Move `options.typeCheck` to the root configuration file.")
}

fn nested_deny_warnings_not_supported(path: &Path) -> OxcDiagnostic {
    OxcDiagnostic::error(format!(
        "The `options.denyWarnings` option is only supported in the root config, but it was found in {}.",
        path.display()
    ))
    .with_help("Move `options.denyWarnings` to the root configuration file.")
}

fn nested_max_warnings_not_supported(path: &Path) -> OxcDiagnostic {
    OxcDiagnostic::error(format!(
        "The `options.maxWarnings` option is only supported in the root config, but it was found in {}.",
        path.display()
    ))
    .with_help("Move `options.maxWarnings` to the root configuration file.")
}

fn nested_report_unused_disable_directives_not_supported(path: &Path) -> OxcDiagnostic {
    OxcDiagnostic::error(format!(
        "The `options.reportUnusedDisableDirectives` option is only supported in the root config, but it was found in {}.",
        path.display()
    ))
    .with_help("Move `options.reportUnusedDisableDirectives` to the root configuration file.")
}

fn nested_respect_eslint_disable_directives_not_supported(path: &Path) -> OxcDiagnostic {
    OxcDiagnostic::error(format!(
        "The `options.respectEslintDisableDirectives` option is only supported in the root config, but it was found in {}.",
        path.display()
    ))
    .with_help("Move `options.respectEslintDisableDirectives` to the root configuration file.")
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        process,
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::discover_configs_in_tree;

    static NEXT_TEST_DIR_ID: AtomicU64 = AtomicU64::new(0);

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(prefix: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "oxc-ark-lint-config-{prefix}-{}-{}",
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

    #[test]
    fn discover_configs_in_tree_skips_package_modules() {
        let dir = TestDir::new("package-modules");
        fs::create_dir_all(dir.join("src")).expect("src dir should be created");
        fs::create_dir_all(dir.join("node_modules/pkg"))
            .expect("node_modules dir should be created");
        fs::create_dir_all(dir.join("oh_modules/pkg")).expect("oh_modules dir should be created");

        let base_config_path = dir.join(".oxlintrc.json");
        fs::write(&base_config_path, r#"{"rules":{}}"#).expect("base config should be written");
        fs::write(dir.join("src/.oxlintrc.json"), r#"{"rules":{}}"#)
            .expect("src config should be written");
        fs::write(
            dir.join("node_modules/pkg/.oxlintrc.json"),
            r#"{"rules":{}}"#,
        )
        .expect("node_modules config should be written");
        fs::write(dir.join("oh_modules/pkg/.oxlintrc.json"), r#"{"rules":{}}"#)
            .expect("oh_modules config should be written");

        let mut configs = discover_configs_in_tree(dir.path(), &base_config_path)
            .into_iter()
            .map(|config| {
                config
                    .path()
                    .strip_prefix(dir.path())
                    .expect("config should be under temp root")
                    .to_string_lossy()
                    .to_string()
            })
            .collect::<Vec<_>>();
        configs.sort();

        assert_eq!(configs, vec!["src/.oxlintrc.json"]);
    }
}
