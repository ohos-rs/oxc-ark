// Portions of this file are derived from Oxc's oxfmt implementation.
// Copyright (c) Oxc project contributors.
// Licensed under the MIT License. See https://github.com/oxc-project/oxc/blob/main/LICENSE.

use std::path::{Path, PathBuf};

use oxc_formatter::JsFormatOptions as FormatOptions;
use oxc_formatter_json::JsonFormatOptions;
use oxc_toml::Options as TomlFormatterOptions;
use serde_json::Value;

use super::FormatFileStrategy;

/// Resolve config file path from cwd and optional explicit path.
pub fn resolve_oxfmtrc_path(cwd: &Path, config_path: Option<&Path>) -> Option<PathBuf> {
    // If `--config` is explicitly specified, use that path (aligned with oxfmt)
    if let Some(config_path) = config_path {
        return Some(super::utils::normalize_relative_path(cwd, config_path));
    }

    // If `--config` is not specified, search the nearest config file from cwd upwards
    // Support both `.json` and `.jsonc`, but prefer `.json` if both exist
    cwd.ancestors().find_map(|dir| {
        for filename in [".oxfmtrc.json", ".oxfmtrc.jsonc"] {
            let config_path = dir.join(filename);
            if config_path.exists() {
                return Some(config_path);
            }
        }
        None
    })
}

pub fn resolve_editorconfig_path(cwd: &Path) -> Option<PathBuf> {
    // Search the nearest `.editorconfig` from cwd upwards
    cwd.ancestors()
        .map(|dir| dir.join(".editorconfig"))
        .find(|p| p.exists())
}

// ---

/// Resolved options for each file type.
/// Each variant contains only the options needed for that formatter.
pub enum ResolvedOptions {
    Upstream(crate::core::format::FormatPlan),
    /// For JS/TS files formatted by oxc_formatter.
    OxcFormatter {
        format_options: Box<FormatOptions>,
        /// For embedded language formatting (e.g., CSS in template literals)
        external_options: Value,
        insert_final_newline: bool,
    },
    /// For TOML files.
    OxfmtToml {
        toml_options: TomlFormatterOptions,
        insert_final_newline: bool,
    },
    /// For JSON/JSON5/JSONC files.
    OxfmtJson {
        json_options: JsonFormatOptions,
        insert_final_newline: bool,
    },
    /// For non-JS files formatted by external formatter (Prettier).
    #[cfg(feature = "napi")]
    ExternalFormatter {
        external_options: Value,
        insert_final_newline: bool,
    },
    /// For `package.json` files: optionally sorted then formatted.
    #[cfg(feature = "napi")]
    ExternalFormatterPackageJson {
        external_options: Value,
        sort_package_json: bool,
        insert_final_newline: bool,
    },
}

/// Configuration resolver that derives all config values from a single `serde_json::Value`.
pub struct ConfigResolver {
    /// User's raw config as JSON value.
    raw_config: Value,
    base_dir: Option<PathBuf>,
    editorconfig: Option<editorconfig_parser::EditorConfig>,
    overrides: Option<crate::core::config::overrides::OxfmtrcOverrides>,
    /// Cached parsed options after validation.
    upstream: Option<(
        std::sync::Arc<crate::core::oxfmtrc::FormatConfig>,
        std::sync::Arc<crate::core::options::ValidatedOptions>,
    )>,
}

impl ConfigResolver {
    /// Create a new resolver from a raw JSON config value.
    pub fn from_value(raw_config: Value) -> Self {
        Self {
            raw_config,
            base_dir: None,
            editorconfig: None,
            overrides: None,
            upstream: None,
        }
    }

    /// Resolve CLI values with EditorConfig when no formatter config file exists.
    pub fn from_cli_options(
        cwd: &Path,
        options: Value,
        editorconfig_path: Option<&Path>,
    ) -> Result<Self, String> {
        let mut resolver = Self::from_config_paths(cwd, None, editorconfig_path)?;
        resolver.raw_config = options;
        Ok(resolver)
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
        // Read and parse config file, or use empty JSON if not found
        let json_string = match oxfmtrc_path {
            Some(path) => {
                let mut json_string = super::utils::read_to_string(path)
                    .map_err(|_| format!("Failed to read {}: File not found", path.display()))?;
                // Strip comments (JSONC support)
                json_strip_comments::strip(&mut json_string).map_err(|err| {
                    format!("Failed to strip comments from {}: {err}", path.display())
                })?;
                json_string
            }
            None => "{}".to_string(),
        };

        // Parse as raw JSON value
        let raw_config: Value = serde_json::from_str(&json_string)
            .map_err(|err| format!("Failed to parse config: {err}"))?;

        Ok(Self {
            raw_config,
            base_dir: Some(
                oxfmtrc_path
                    .and_then(Path::parent)
                    .unwrap_or(cwd)
                    .to_path_buf(),
            ),
            editorconfig: crate::core::config::editorconfig::load_editorconfig(editorconfig_path)?,
            overrides: None,
            upstream: None,
        })
    }

    /// Validate config and return ignore patterns for file walking.
    ///
    /// Validated options are cached for fast path resolution.
    ///
    /// # Errors
    /// Returns error if config deserialization fails.
    pub fn build_and_validate(&mut self) -> Result<Vec<String>, String> {
        let mut upstream: crate::core::oxfmtrc::Oxfmtrc =
            serde_json::from_value(self.raw_config.clone())
                .map_err(|err| format!("Failed to deserialize Oxfmtrc: {err}"))?;
        if let Some(base) = &self.base_dir {
            upstream.format_config.resolve_tailwind_paths(base);
        }
        let overrides = upstream.overrides.take().unwrap_or_default();
        for item in &overrides {
            crate::core::options::validate(&item.options)?;
        }
        self.overrides = Some(crate::core::config::overrides::OxfmtrcOverrides::new(
            overrides,
            self.base_dir.clone(),
        ));
        let validated = crate::core::options::validate(&upstream.format_config)?;
        self.upstream = Some((
            std::sync::Arc::new(upstream.format_config),
            std::sync::Arc::new(validated),
        ));
        let ignore_patterns =
            serde_json::from_value::<crate::core::oxfmtrc::Oxfmtrc>(self.raw_config.clone())
                .map_err(|err| err.to_string())?
                .ignore_patterns
                .unwrap_or_default();
        Ok(ignore_patterns)
    }

    /// Resolve validated upstream options for a file.
    pub fn resolve(&self, strategy: &FormatFileStrategy) -> Result<ResolvedOptions, String> {
        let (config, _validated) = self.upstream.as_ref().expect("validated config");
        let path = strategy.path();
        let mut native = crate::core::support::classify_file_with_language(path, None)
            .ok_or_else(|| format!("Unsupported file type: {}", path.display()))?;
        if let FormatFileStrategy::OxcFormatter { source_type, .. } = strategy {
            native = crate::core::support::FormatStrategy::Native(
                crate::core::support::NativeLanguage::Js(*source_type),
            );
        }
        let mut resolved = (**config).clone();
        for options in self
            .overrides
            .as_ref()
            .expect("validated overrides")
            .matching(path)
        {
            resolved.merge(options);
        }
        if let Some(editorconfig) = &self.editorconfig {
            crate::core::config::editorconfig::apply_editorconfig(
                &mut resolved,
                &editorconfig.resolve(path),
            );
        }
        let validated = std::sync::Arc::new(crate::core::options::validate(&resolved)?);
        #[cfg(feature = "napi")]
        if let crate::core::support::FormatStrategy::Prettier(language) = &native
            && let Some(plugin) = language.missing_plugin(&resolved)
        {
            return Err(format!(
                "Enable `{plugin}` in formatter options to format {}",
                path.display()
            ));
        }
        Ok(ResolvedOptions::Upstream(crate::core::format::FormatPlan {
            path: path.to_path_buf().into(),
            strategy: native,
            config: std::sync::Arc::new(resolved),
            validated,
        }))
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        process,
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::{ConfigResolver, resolve_oxfmtrc_path};

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
        assert_eq!(
            resolver.upstream.as_ref().unwrap().0.single_quote,
            Some(true)
        );
    }
}
