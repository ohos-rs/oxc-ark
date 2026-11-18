// Derived from Oxc's oxfmt implementation, with ArkTS/ETS and OXK-specific behavior preserved.
// Copyright (c) Oxc project contributors.
// Licensed under the MIT License. See https://github.com/oxc-project/oxc/blob/main/LICENSE.

mod associations;
mod editorconfig;
#[cfg(feature = "napi")]
mod js_config;
mod nested;
mod overrides;
mod scopes;

#[cfg(feature = "napi")]
pub use js_config::{JsConfigLoaderCb, JsLoadJsConfigCb, create_js_config_loader};

/// JS config loading is only available in the NAPI build. The alias keeps the
/// `ConfigScopes` signature stable when feature unification enables `napi` for
/// some workspace members but not others.
#[cfg(not(feature = "napi"))]
pub type JsConfigLoaderCb =
    std::sync::Arc<dyn Fn(String) -> Result<serde_json::Value, String> + Send + Sync>;
pub use nested::NestedConfigCtx;
pub use scopes::ConfigScopes;

use std::{
    borrow::Cow,
    path::{Path, PathBuf},
    sync::Arc,
};

use editorconfig_parser::EditorConfig;
#[cfg(not(target_family = "wasm"))]
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use serde_json::Value;
use tracing::instrument;

use oxc_config::{ConfigDiscovery, DiscoveredConfigFile, is_js_config_path, vp_version};

use crate::{FormatFileStrategy, ResolvedOptions};

use self::{
    associations::OxfmtrcAssociations,
    editorconfig::{apply_editorconfig, resolve_editorconfig_overrides, root_properties},
    overrides::OxfmtrcOverrides,
};
#[cfg(not(target_family = "wasm"))]
use super::global_ignore::matches_with_ancestors;
use super::{
    Language,
    format::FormatPlan,
    options::{ValidatedOptions, validate},
    oxfmtrc::{FormatConfig, Oxfmtrc},
    support::{FormatStrategy, classify_as, classify_file, classify_file_with_language},
    utils,
};

/// `path` relative to the config directory, as written glob patterns expect.
fn relative_path<'a>(base_dir: Option<&Path>, path: &'a Path) -> Cow<'a, str> {
    // NOTE: On Windows, `to_string_lossy()` produces `\`-separated paths.
    // This is OK since `fast_glob::glob_match()` supports both `/` and `\` via `std::path::is_separator`.
    base_dir
        .and_then(|dir| path.strip_prefix(dir).ok())
        .unwrap_or(path)
        .to_string_lossy()
}

pub fn config_discovery() -> ConfigDiscovery {
    if cfg!(feature = "napi") && vp_version().is_some() {
        ConfigDiscovery::vite_plus()
    } else {
        ConfigDiscovery::oxfmt()
    }
}

/// Everything a config file load needs besides the file itself,
/// shared by the root load ([`ConfigScopes::load`]) and nested probes ([`NestedConfigCtx`]).
///
/// Cloning is shallow.
#[derive(Clone)]
struct ConfigLoader {
    discovery: ConfigDiscovery,
    /// Parsed `.editorconfig`, shared by every resolver this loads,
    /// instead of re-reading and re-parsing the same file.
    editorconfig: Option<Arc<EditorConfig>>,
    #[cfg(feature = "napi")]
    js_loader: Option<JsConfigLoaderCb>,
}

impl ConfigLoader {
    #[cfg_attr(not(feature = "napi"), allow(unused_variables))]
    fn new(editorconfig: Option<EditorConfig>, js_loader: Option<JsConfigLoaderCb>) -> Self {
        Self {
            discovery: config_discovery(),
            editorconfig: editorconfig.map(Arc::new),
            #[cfg(feature = "napi")]
            js_loader,
        }
    }

    /// Load the root config, handling both JSON/JSONC and JS/TS config files.
    ///
    /// When `explicit_config` is `Some`, it is treated as an explicitly specified config file.
    /// When `explicit_config` is `None`, auto-discovery searches upwards from `cwd`,
    /// and falls back to the default (empty) config.
    ///
    /// # Errors
    /// Returns error if config file loading or parsing fails.
    fn load_root(
        &self,
        cwd: &Path,
        explicit_config: Option<&Path>,
        fallback_config: Option<Value>,
    ) -> Result<ConfigResolver, String> {
        // Explicit path: normalize and load directly
        if let Some(config_path) = explicit_config {
            let path = utils::normalize_relative_path(cwd, config_path);
            if !is_js_config_path(&path) {
                return ConfigResolver::from_json_config(Some(&path), self.editorconfig.clone());
            }
            let raw_config = self
                .load_js_config(&path)?
                // Explicit `--config`: missing `.fmt` is an error.
                .ok_or_else(|| {
                    format!(
                        "Expected a `fmt` field in the default export of {}",
                        path.display()
                    )
                })?;
            return Ok(self.js_resolver(&path, raw_config));
        }

        // Auto-discovery: search upwards from cwd, load in one pass
        for dir in cwd.ancestors() {
            if let Some(resolver) = self.load_in_dir(dir)? {
                return Ok(resolver);
            }
        }

        // No config found: apply OXK's CLI options when provided, otherwise use defaults.
        match fallback_config {
            Some(raw_config) => Ok(ConfigResolver::new(
                raw_config,
                Some(cwd.to_path_buf()),
                self.editorconfig.clone(),
            )),
            None => ConfigResolver::from_json_config(None, self.editorconfig.clone()),
        }
    }

    /// Load a config file located directly inside `dir` (no `build_and_validate`).
    ///
    /// Returns `Ok(None)` when `dir` has no config file,
    /// or the file is a `vite.config.*` whose default export lacks a `.fmt` field.
    fn load_in_dir(&self, dir: &Path) -> Result<Option<ConfigResolver>, String> {
        // Preserve OXK's historical preference for `.oxfmtrc.json` over `.oxfmtrc.jsonc`
        // when both exist in the same directory.
        for file_name in [".oxfmtrc.json", ".oxfmtrc.jsonc"] {
            let path = dir.join(file_name);
            if path.is_file() {
                return ConfigResolver::from_json_config(Some(&path), self.editorconfig.clone())
                    .map(Some);
            }
        }

        let Some(config_file) = self
            .discovery
            .find_unique_config_by_readdir(dir)
            .map_err(|e| Into::<oxc_diagnostics::OxcDiagnostic>::into(e).to_string())?
        else {
            return Ok(None);
        };

        let (path, raw_config) = match config_file {
            DiscoveredConfigFile::Json(path) | DiscoveredConfigFile::Jsonc(path) => {
                return ConfigResolver::from_json_config(Some(&path), self.editorconfig.clone())
                    .map(Some);
            }
            DiscoveredConfigFile::Js(path) => {
                let raw_config = self
                    .load_js_config(&path)?
                    .expect("loadJsConfig never returns null for non-Vite JS config");
                (path, raw_config)
            }
            DiscoveredConfigFile::Vite(path) => {
                let Some(raw_config) = self.load_js_config(&path)? else {
                    return Ok(None);
                };
                (path, raw_config)
            }
        };
        Ok(Some(self.js_resolver(&path, raw_config)))
    }

    fn js_resolver(&self, path: &Path, raw_config: Value) -> ConfigResolver {
        ConfigResolver::new(
            raw_config,
            path.parent().map(Path::to_path_buf),
            self.editorconfig.clone(),
        )
    }
}

#[cfg(feature = "napi")]
impl ConfigLoader {
    /// Load a JS/TS config file via NAPI and return the raw JSON value.
    ///
    /// Returns `Ok(None)` when the JS side returns `null` (Vite+ `.fmt` missing).
    fn load_js_config(&self, path: &Path) -> Result<Option<Value>, String> {
        let Some(js_config_loader) = self.js_loader.as_ref() else {
            return Err(format!(
                "JS/TS config file ({}) is not supported without a JS config loader.\nUse JSON/JSONC instead.",
                path.display()
            ));
        };
        let value = js_config_loader(path.to_string_lossy().into_owned()).map_err(|err| {
            format!(
                "{}\n{err}\nEnsure the file has a valid default export of a JSON-serializable configuration object.",
                path.display()
            )
        })?;

        Ok(if value.is_null() { None } else { Some(value) })
    }
}

#[cfg(not(feature = "napi"))]
impl ConfigLoader {
    /// JS/TS config files need the Node.js CLI.
    #[expect(clippy::unused_self)]
    fn load_js_config(&self, path: &Path) -> Result<Option<Value>, String> {
        Err(format!(
            "JS/TS config file ({}) is not supported in pure Rust CLI.\nUse JSON/JSONC instead.",
            path.display()
        ))
    }
}

// ---

/// Outcome of resolving a [`FormatStrategy`] against a [`FormatConfig`].
#[derive(Debug)]
pub enum ResolveOutcome {
    /// Ready to format with this plan.
    Format(FormatPlan),
    /// The file's parser requires a plugin that the resolved config did NOT enable.
    /// The payload carries the missing config key (e.g. `"svelte"`).
    MissingPlugin(&'static str),
}

/// Apply the missing-plugin gate, then build the [`ResolveOutcome`].
fn into_outcome(
    config: Arc<FormatConfig>,
    validated: Arc<ValidatedOptions>,
    path: &Path,
    strategy: FormatStrategy,
) -> ResolveOutcome {
    #[cfg(feature = "napi")]
    if let FormatStrategy::Prettier(language) = &strategy
        && let Some(plugin) = language.missing_plugin(&config)
    {
        return ResolveOutcome::MissingPlugin(plugin);
    }
    ResolveOutcome::Format(FormatPlan {
        path: Arc::from(path),
        strategy,
        config,
        validated,
    })
}

/// Resolve options for a pre-classified file and build a [`ResolveOutcome`].
///
/// This is the simplified path for the NAPI `format()` API.
/// Relative Tailwind paths are resolved against the provided `cwd`.
#[cfg(feature = "napi")]
pub fn resolve_for_api(
    raw_config: Value,
    path: &Path,
    strategy: FormatStrategy,
    cwd: &Path,
) -> Result<ResolveOutcome, String> {
    let mut format_config: FormatConfig =
        serde_json::from_value(raw_config).map_err(|err| err.to_string())?;
    format_config.resolve_tailwind_paths(cwd);
    let validated = validate(&format_config)?;
    Ok(into_outcome(
        Arc::new(format_config),
        Arc::new(validated),
        path,
        strategy,
    ))
}

// ---

/// Configuration resolver to handle `.oxfmtrc` and `.editorconfig` files.
///
/// Priority (later wins):
/// - `.editorconfig` (fallback for unset fields)
/// - `.oxfmtrc` base
/// - `.oxfmtrc` overrides matching the file path.
#[derive(Debug)]
pub struct ConfigResolver {
    /// User's raw config as JSON value.
    ///
    /// Retained because the slow path must re-deserialize [`FormatConfig`] from it,
    /// so per-file overrides can override values that the `[*]` editorconfig section baked in.
    raw_config: Value,
    /// Directory containing the config file (for relative path resolution).
    config_dir: Option<PathBuf>,
    /// Fast-path snapshot: the typed `FormatConfig` (base + `.editorconfig` `[*]`)
    /// paired with its validation-gate artifacts.
    base: Option<(Arc<FormatConfig>, Arc<ValidatedOptions>)>,
    /// Resolved overrides from `.oxfmtrc` for file-specific matching.
    oxfmtrc_overrides: Option<OxfmtrcOverrides>,
    /// Resolved `associations` from `.oxfmtrc`, consulted before built-in classification.
    associations: Option<OxfmtrcAssociations>,
    /// Ignore glob built from this config's `ignorePatterns`.
    #[cfg(not(target_family = "wasm"))]
    ignore_glob: Option<Gitignore>,
    /// Parsed `.editorconfig`, if any.
    editorconfig: Option<Arc<EditorConfig>>,
}

impl ConfigResolver {
    /// Shared internal constructor used by both:
    /// - `from_json_config()` (JSON/JSONC)
    /// - and [`ConfigLoader`] (JS/TS config evaluated externally)
    fn new(
        raw_config: Value,
        config_dir: Option<PathBuf>,
        editorconfig: Option<Arc<EditorConfig>>,
    ) -> Self {
        Self {
            raw_config,
            config_dir,
            base: None,
            oxfmtrc_overrides: None,
            associations: None,
            #[cfg(not(target_family = "wasm"))]
            ignore_glob: None,
            editorconfig,
        }
    }

    /// Returns the directory containing the config file, if any was loaded.
    pub fn config_dir(&self) -> Option<&Path> {
        self.config_dir.as_deref()
    }

    /// Returns `true` if the given path should be ignored by this config's `ignorePatterns`.
    pub fn is_path_ignored(&self, path: &Path, is_dir: bool) -> bool {
        #[cfg(not(target_family = "wasm"))]
        {
            self.ignore_glob
                .as_ref()
                .is_some_and(|glob| matches_with_ancestors(glob, path, is_dir))
        }
        #[cfg(target_family = "wasm")]
        {
            let _ = (path, is_dir);
            false
        }
    }

    /// Create a resolver from a raw JSON config value.
    pub fn from_value(raw_config: Value) -> Self {
        Self::new(raw_config, None, None)
    }

    /// Resolve CLI values with EditorConfig when no formatter config file exists.
    pub fn from_cli_options(
        cwd: &Path,
        options: Value,
        editorconfig_path: Option<&Path>,
    ) -> Result<Self, String> {
        let editorconfig = editorconfig::load_editorconfig(editorconfig_path)?.map(Arc::new);
        // `cwd` is the base for relative Tailwind paths in CLI-only runs.
        Ok(Self::new(options, Some(cwd.to_path_buf()), editorconfig))
    }

    /// Create a resolver by loading config from a file path.
    ///
    /// # Errors
    /// Returns error if:
    /// - Config file is specified but not found or invalid
    /// - Config file parsing fails
    pub fn from_config_paths(
        cwd: &Path,
        oxfmtrc_path: Option<&Path>,
        editorconfig_path: Option<&Path>,
    ) -> Result<Self, String> {
        let editorconfig = editorconfig::load_editorconfig(editorconfig_path)?.map(Arc::new);
        let config_path = oxfmtrc_path.map(|path| utils::normalize_relative_path(cwd, path));
        if let Some(path) = &config_path
            && is_js_config_path(path)
        {
            return Err(format!(
                "JS/TS config file ({}) is not supported by `from_config_paths`.\nUse `ConfigScopes` with a JS config loader instead.",
                path.display()
            ));
        }
        Self::from_json_config(config_path.as_deref(), editorconfig)
    }

    /// Create a resolver from a JSON/JSONC config path.
    ///
    /// Also used as the default (empty config) fallback when no config file is found.
    #[instrument(level = "debug", name = "oxfmt::config::from_json_config", skip_all)]
    fn from_json_config(
        oxfmtrc_path: Option<&Path>,
        editorconfig: Option<Arc<EditorConfig>>,
    ) -> Result<Self, String> {
        // Read and parse config file, or use empty JSON if not found
        let json_string = match oxfmtrc_path {
            Some(path) => {
                let mut json_string = utils::read_to_string(path)
                    // Do not include OS error, it differs between platforms
                    .map_err(|_| format!("Failed to read {}: File not found", path.display()))?;
                // Strip BOM, editors on Windows may add it
                if json_string.starts_with('\u{feff}') {
                    json_string.drain(..'\u{feff}'.len_utf8());
                }
                // Strip comments (JSONC support)
                json_strip_comments::strip(&mut json_string).map_err(|err| {
                    format!("Failed to strip comments from {}: {err}", path.display())
                })?;
                json_string
            }
            None => "{}".to_string(),
        };

        // Parse as raw JSON value
        let raw_config: Value =
            serde_json::from_str(&json_string).map_err(|err| err.to_string())?;
        let config_dir = oxfmtrc_path.and_then(|p| p.parent().map(Path::to_path_buf));

        Ok(Self::new(raw_config, config_dir, editorconfig))
    }

    /// Validate config and build the ignore glob from `ignorePatterns` for file walking.
    ///
    /// Returns the raw `ignorePatterns` list for backwards compatibility with callers
    /// that build their own matchers.
    ///
    /// # Errors
    /// Returns error if config deserialization or validation fails.
    #[instrument(level = "debug", name = "oxfmt::config::build_and_validate", skip_all)]
    pub fn build_and_validate(&mut self) -> Result<Vec<String>, String> {
        let oxfmtrc: Oxfmtrc =
            serde_json::from_value(self.raw_config.clone()).map_err(|err| err.to_string())?;

        // Resolve `overrides` and `associations` for later per-file matching
        self.oxfmtrc_overrides = oxfmtrc
            .overrides
            .map(|overrides| OxfmtrcOverrides::new(overrides, self.config_dir.clone()));
        self.associations = oxfmtrc
            .associations
            .filter(|associations| !associations.is_empty())
            .map(|associations| OxfmtrcAssociations::new(associations, self.config_dir.clone()));

        let mut format_config = oxfmtrc.format_config;

        // Apply `.editorconfig` root section now.
        // Per-file sections are deferred to the slow path during `resolve_options()`.
        if let Some(editorconfig) = &self.editorconfig
            && let Some(props) = root_properties(editorconfig)
        {
            apply_editorconfig(&mut format_config, props);
        }

        if let Some(config_dir) = &self.config_dir {
            format_config.resolve_tailwind_paths(config_dir);
        }

        // Eagerly validate; the snapshot and its gate artifacts are cached as one pair.
        let validated = validate(&format_config)?;
        self.base = Some((Arc::new(format_config), Arc::new(validated)));

        // Build ignore glob from `ignorePatterns` config field
        let ignore_patterns = oxfmtrc.ignore_patterns.unwrap_or_default();
        #[cfg(not(target_family = "wasm"))]
        {
            self.ignore_glob = build_ignore_glob(self.config_dir.as_deref(), &ignore_patterns)?;
        }

        Ok(ignore_patterns)
    }

    /// Classify `path`: the given `language` first (e.g. LSP `languageId`),
    /// then this config's `associations`, then the file name.
    pub fn classify(&self, path: &Path, language: Option<Language>) -> Option<FormatStrategy> {
        let language = language.or_else(|| self.associations.as_ref()?.matching(path));
        match language {
            Some(language) => classify_as(language, path),
            None => classify_file(path),
        }
    }

    /// Classify `path`, preferring an explicit language mode (e.g. `ets-static`).
    ///
    /// Explicit modes only override the inputs they apply to (currently `.ets`),
    /// preserving inference for the rest of a mixed project.
    pub fn classify_with_explicit_language(
        &self,
        path: &Path,
        language: Option<oxc_span::ExplicitLanguage>,
    ) -> Option<FormatStrategy> {
        if language.is_some() {
            classify_file_with_language(path, language)
        } else {
            self.classify(path, None)
        }
    }

    /// Resolve options for a pre-classified file and build a [`ResolveOutcome`].
    pub fn resolve_outcome(
        &self,
        path: &Path,
        strategy: FormatStrategy,
    ) -> Result<ResolveOutcome, String> {
        let (format_config, validated) = self.resolve_options(path)?;
        Ok(into_outcome(format_config, validated, path, strategy))
    }

    /// Resolve options for a path, applying `associations` and the explicit language mode.
    ///
    /// This is the entry point for callers that start from a path rather than
    /// a pre-classified [`FormatStrategy`]. It returns [`ResolvedOptions`] for the
    /// existing formatter pipeline while still applying associations.
    pub fn resolve_for_path(
        &self,
        path: &Path,
        language: Option<oxc_span::ExplicitLanguage>,
    ) -> Result<ResolvedOptions, String> {
        let strategy = self
            .classify_with_explicit_language(path, language)
            .ok_or_else(|| format!("Unsupported file type: {}", path.display()))?;
        let (format_config, validated) = self.resolve_options(path)?;
        #[cfg(feature = "napi")]
        if let FormatStrategy::Prettier(language) = &strategy
            && let Some(plugin) = language.missing_plugin(&format_config)
        {
            return Err(format!(
                "Enable `{plugin}` in formatter options to format {}",
                path.display()
            ));
        }
        Ok(ResolvedOptions::Upstream(FormatPlan {
            path: Arc::from(path),
            strategy,
            config: format_config,
            validated,
        }))
    }

    /// Resolve options from an already-classified legacy [`FormatFileStrategy`].
    ///
    /// Kept for the CLI/NAPI entry points that classify before resolving.
    pub fn resolve(&self, strategy: &FormatFileStrategy) -> Result<ResolvedOptions, String> {
        let path = strategy.path();
        let native = self.resolve_legacy_strategy(path, strategy)?;
        let (format_config, validated) = self.resolve_options(path)?;
        #[cfg(feature = "napi")]
        if let FormatStrategy::Prettier(language) = &native
            && let Some(plugin) = language.missing_plugin(&format_config)
        {
            return Err(format!(
                "Enable `{plugin}` in formatter options to format {}",
                path.display()
            ));
        }
        Ok(ResolvedOptions::Upstream(FormatPlan {
            path: path.to_path_buf().into(),
            strategy: native,
            config: format_config,
            validated,
        }))
    }

    /// Map a legacy [`FormatFileStrategy`] to the core [`FormatStrategy`],
    /// applying `associations` unless an explicit `ets-static` mode was selected.
    fn resolve_legacy_strategy(
        &self,
        path: &Path,
        strategy: &FormatFileStrategy,
    ) -> Result<FormatStrategy, String> {
        // An explicit `ets-static` source type always wins.
        if let FormatFileStrategy::OxcFormatter { source_type, .. } = strategy
            && source_type.is_ets_static()
        {
            return Ok(FormatStrategy::Native(super::support::NativeLanguage::Js(
                *source_type,
            )));
        }

        // Associations take precedence over the file-name classification.
        if let Some(language) = self.associations.as_ref().and_then(|a| a.matching(path)) {
            return classify_as(language, path)
                .ok_or_else(|| format!("Unsupported file type: {}", path.display()));
        }

        match strategy {
            FormatFileStrategy::OxcFormatter { source_type, .. } => Ok(FormatStrategy::Native(
                super::support::NativeLanguage::Js(*source_type),
            )),
            _ => classify_file_with_language(path, None)
                .ok_or_else(|| format!("Unsupported file type: {}", path.display())),
        }
    }

    /// Resolve `FormatConfig` for a specific file path.
    ///
    /// Priority (later wins):
    /// - `.editorconfig` (fallback for unset fields)
    /// - `.oxfmtrc` base
    /// - `.oxfmtrc` overrides matching the file path
    ///
    /// Fast path: reuses the snapshot + gate artifacts cached by [`Self::build_and_validate`].
    /// Slow path: rebuilds from the raw config and validates the merged config.
    fn resolve_options(
        &self,
        path: &Path,
    ) -> Result<(Arc<FormatConfig>, Arc<ValidatedOptions>), String> {
        let oxfmtrc_overrides = self
            .oxfmtrc_overrides
            .as_ref()
            .map_or_else(Vec::new, |o| o.matching(path));
        // `.editorconfig` `[*]` is already folded in during `build_and_validate()`,
        // so only a per-file section that changes the result counts as an override.
        let editorconfig_overrides = self
            .editorconfig
            .as_ref()
            .and_then(|ec| resolve_editorconfig_overrides(ec, path));

        // Fast path: no per-file overrides → share the cached (already-validated) snapshot.
        if oxfmtrc_overrides.is_empty() && editorconfig_overrides.is_none() {
            let (config, validated) = self
                .base
                .as_ref()
                .expect("`build_and_validate()` must be called first");
            return Ok((Arc::clone(config), Arc::clone(validated)));
        }

        // Slow path: rebuild from `raw_config`, NOT from the cached snapshot.
        let mut format_config: FormatConfig = serde_json::from_value(self.raw_config.clone())
            .expect("`build_and_validate()` should catch this before");

        // Apply oxfmtrc overrides first (explicit settings)
        for options in oxfmtrc_overrides {
            format_config.merge(options);
        }
        // Apply `.editorconfig` as fallback (fills in unset fields only).
        if let Some(ec) = &self.editorconfig
            && let Some(props) = editorconfig_overrides
                .as_ref()
                .or_else(|| root_properties(ec))
        {
            apply_editorconfig(&mut format_config, props);
        }

        if let Some(config_dir) = &self.config_dir {
            format_config.resolve_tailwind_paths(config_dir);
        }

        let validated = validate(&format_config)?;
        Ok((Arc::new(format_config), Arc::new(validated)))
    }
}

/// Build an ignore glob from config `ignorePatterns`.
/// Patterns are resolved relative to the config file's directory.
#[cfg(not(target_family = "wasm"))]
fn build_ignore_glob(
    config_dir: Option<&Path>,
    ignore_patterns: &[String],
) -> Result<Option<Gitignore>, String> {
    if ignore_patterns.is_empty() {
        return Ok(None);
    }
    let Some(config_dir) = config_dir else {
        return Ok(None);
    };

    let mut builder = GitignoreBuilder::new(config_dir);
    for pattern in ignore_patterns {
        oxc_config::validate_ignore_pattern(pattern)?;

        if builder.add_line(None, pattern).is_err() {
            return Err(format!(
                "Failed to add ignore pattern `{pattern}` from `ignorePatterns`"
            ));
        }
    }
    let gitignore = builder
        .build()
        .map_err(|_| "Failed to build ignores".to_string())?;
    Ok(Some(gitignore))
}

// ---

#[cfg(test)]
mod tests_config_resolution {
    use std::{
        fs,
        path::{Path, PathBuf},
        process,
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::{ConfigResolver, ResolveOutcome};
    use crate::core::support::{FormatStrategy, NativeLanguage};
    use crate::resolve_oxfmtrc_path;

    static NEXT_TEST_DIR_ID: AtomicU64 = AtomicU64::new(0);

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(prefix: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "oxc-ark-config-{prefix}-{}-{}",
                process::id(),
                NEXT_TEST_DIR_ID.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).expect("test temp dir should be created");
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

    fn resolver_from_json(raw: serde_json::Value) -> ConfigResolver {
        let mut resolver = ConfigResolver::new(raw, None, None);
        resolver
            .build_and_validate()
            .expect("base config must be valid for these tests");
        resolver
    }

    #[test]
    fn resolve_oxfmtrc_path_prefers_json_over_jsonc() {
        let dir = TestDir::new("prefer-json");
        let nested_dir = dir.join("nested");
        let json_path = dir.join(".oxfmtrc.json");
        let jsonc_path = dir.join(".oxfmtrc.jsonc");

        fs::create_dir_all(&nested_dir).expect("nested dir should be created");
        fs::write(&json_path, r#"{"singleQuote": true}"#).expect(".json config should be written");
        fs::write(
            &jsonc_path,
            "{\n  // comment\n  \"singleQuote\": false\n}\n",
        )
        .expect(".jsonc config should be written");

        let resolved = resolve_oxfmtrc_path(&nested_dir, None);

        assert_eq!(resolved, Some(json_path));
    }

    #[test]
    fn from_config_paths_parses_jsonc_with_comments() {
        let dir = TestDir::new("parse-jsonc");
        let config_path = dir.join(".oxfmtrc.jsonc");

        fs::write(
            &config_path,
            r#"{
  // JSONC config should be accepted
  "singleQuote": true,
  "ignorePatterns": ["dist/**"]
}"#,
        )
        .expect(".jsonc config should be written");

        let mut resolver = ConfigResolver::from_config_paths(dir.path(), Some(&config_path), None)
            .expect("jsonc config should load");
        let ignore_patterns = resolver
            .build_and_validate()
            .expect("jsonc config should validate");
        assert_eq!(ignore_patterns, vec!["dist/**".to_string()]);
        assert_eq!(resolver.base.as_ref().unwrap().0.single_quote, Some(true));
    }

    #[test]
    fn from_config_paths_strips_bom() {
        let dir = TestDir::new("bom");
        let config_path = dir.join(".oxfmtrc.json");
        fs::write(&config_path, "\u{feff}{\"semi\": false}").expect("config should be written");

        let mut resolver = ConfigResolver::from_config_paths(dir.path(), Some(&config_path), None)
            .expect("config should load");
        resolver
            .build_and_validate()
            .expect("config should validate");
        assert_eq!(resolver.base.as_ref().unwrap().0.semi, Some(false));
    }

    /// Invalid override values are caught at resolve time (slow path),
    /// since the format step itself is infallible.
    #[test]
    fn override_only_invalid_value_is_rejected_for_oxc_formatter() {
        let resolver = resolver_from_json(serde_json::json!({
            "tabWidth": 2,
            "overrides": [
                { "files": ["*.ts"], "options": { "tabWidth": 250 } }
            ]
        }));

        let path = Path::new("src/test.ts");
        let strategy = crate::FormatFileStrategy::OxcFormatter {
            path: path.to_path_buf(),
            source_type: oxc_span::SourceType::ts(),
        };
        let err = resolver.resolve(&strategy).unwrap_err();
        assert!(
            err.contains("tabWidth"),
            "expected tabWidth validation error, got: {err}"
        );
    }

    #[test]
    fn fast_path_resolve_succeeds() {
        let resolver = resolver_from_json(serde_json::json!({ "printWidth": 80 }));
        let path = Path::new("Cargo.toml");
        assert!(
            resolver
                .resolve(&crate::FormatFileStrategy::OxfmtToml {
                    path: path.to_path_buf()
                })
                .is_ok()
        );
    }

    #[test]
    fn associations_pick_language_for_unclassified_extension() {
        let resolver = resolver_from_json(serde_json::json!({
            "associations": [
                { "files": ["*.wxml"], "language": "html" }
            ]
        }));

        let strategy = resolver
            .classify_with_explicit_language(Path::new("page.wxml"), None)
            .expect("association should classify .wxml");
        assert!(matches!(
            strategy,
            FormatStrategy::Native(NativeLanguage::Html(_))
        ));

        let outcome = resolver
            .resolve_outcome(Path::new("page.wxml"), strategy)
            .expect("associations should resolve");
        assert!(matches!(outcome, ResolveOutcome::Format(_)));
    }

    #[test]
    fn later_association_wins() {
        let resolver = resolver_from_json(serde_json::json!({
            "associations": [
                { "files": ["*.conf"], "language": "yaml" },
                { "files": ["*.conf"], "language": "toml" }
            ]
        }));

        let strategy = resolver
            .classify_with_explicit_language(Path::new("app.conf"), None)
            .expect("association should classify .conf");
        assert!(matches!(
            strategy,
            FormatStrategy::Native(NativeLanguage::Toml)
        ));
    }

    #[test]
    fn exclude_files_skip_association() {
        let resolver = resolver_from_json(serde_json::json!({
            "associations": [
                {
                    "files": ["*.conf"],
                    "excludeFiles": ["secret.conf"],
                    "language": "toml"
                }
            ]
        }));

        // Falls back to the built-in classification (unsupported here).
        assert!(
            resolver
                .classify_with_explicit_language(Path::new("secret.conf"), None)
                .is_none()
        );
    }

    #[test]
    fn explicit_ets_static_beats_associations() {
        let resolver = resolver_from_json(serde_json::json!({
            "associations": [
                { "files": ["*.ets"], "language": "typescript" }
            ]
        }));

        let strategy = resolver
            .classify_with_explicit_language(
                Path::new("component.ets"),
                Some(oxc_span::ExplicitLanguage::EtsStatic),
            )
            .expect("ets-static should classify");
        let FormatStrategy::Native(NativeLanguage::Js(source_type)) = strategy else {
            panic!(".ets must use the Oxc formatter");
        };
        assert!(source_type.is_ets_static());
    }
}

#[cfg(all(test, not(target_family = "wasm")))]
mod tests_ignore_patterns_validation {
    use std::path::Path;

    use super::build_ignore_glob;

    fn build(pattern: &str) -> Result<(), String> {
        build_ignore_glob(Some(Path::new("/repo/config")), &[pattern.to_string()]).map(|_| ())
    }

    #[test]
    fn rejects_parent_directory_components() {
        let error = build("../src/skip.js").unwrap_err();
        assert_eq!(
            error,
            "Invalid pattern `../src/skip.js` in `ignorePatterns`: `..` is not supported, patterns are resolved within the config file's directory"
        );
    }

    #[test]
    fn accepts_patterns_without_parent_directory_components() {
        assert!(build("src/skip.js").is_ok());
    }
}
