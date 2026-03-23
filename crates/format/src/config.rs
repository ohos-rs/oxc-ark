use std::path::{Path, PathBuf};

use oxc_formatter::FormatOptions;
use oxc_toml::Options as TomlFormatterOptions;
use serde_json::Value;

use crate::oxfmtrc::{OxfmtOptions, Oxfmtrc, populate_prettier_config};

use super::FormatFileStrategy;
use super::support::JsonType;

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
        json_options: JsonFormatterOptions,
        json_type: JsonType,
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
    /// Cached parsed options after validation.
    cached_options: Option<(OxfmtOptions, Value)>,
}

impl ConfigResolver {
    /// Create a new resolver from a raw JSON config value.
    pub fn from_value(raw_config: Value) -> Self {
        Self {
            raw_config,
            cached_options: None,
        }
    }

    /// Create a resolver by loading config from a file path.
    ///
    /// # Errors
    /// Returns error if:
    /// - Config file is specified but not found or invalid
    /// - Config file parsing fails
    pub fn from_config_paths(
        _cwd: &Path,
        oxfmtrc_path: Option<&Path>,
        _editorconfig_path: Option<&Path>,
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
            cached_options: None,
        })
    }

    /// Validate config and return ignore patterns for file walking.
    ///
    /// Validated options are cached for fast path resolution.
    ///
    /// # Errors
    /// Returns error if config deserialization fails.
    pub fn build_and_validate(&mut self) -> Result<Vec<String>, String> {
        let oxfmtrc: Oxfmtrc = serde_json::from_value(self.raw_config.clone())
            .map_err(|err| format!("Failed to deserialize Oxfmtrc: {err}"))?;

        let oxfmt_options = oxfmtrc
            .format_config
            .into_oxfmt_options()
            .map_err(|err| format!("Failed to parse configuration.\n{err}"))?;

        let ignore_patterns = oxfmtrc.ignore_patterns.clone().unwrap_or_default();

        let mut external_options = self.raw_config.clone();
        populate_prettier_config(&oxfmt_options.format_options, &mut external_options);

        self.cached_options = Some((oxfmt_options, external_options));

        Ok(ignore_patterns)
    }

    /// Resolve format options for a specific file.
    pub fn resolve(&self, strategy: &FormatFileStrategy) -> ResolvedOptions {
        let (oxfmt_options, external_options) = self
            .cached_options
            .clone()
            .expect("`build_and_validate()` must be called before `resolve()`");

        let insert_final_newline = oxfmt_options.insert_final_newline;

        match strategy {
            FormatFileStrategy::OxcFormatter { .. } => ResolvedOptions::OxcFormatter {
                format_options: Box::new(oxfmt_options.format_options),
                external_options,
                insert_final_newline,
            },
            FormatFileStrategy::OxfmtToml { .. } => ResolvedOptions::OxfmtToml {
                toml_options: oxfmt_options.toml_options,
                insert_final_newline,
            },
            FormatFileStrategy::OxfmtJson { json_type, .. } => ResolvedOptions::OxfmtJson {
                json_options: build_json_options(&oxfmt_options.format_options),
                json_type: *json_type,
                insert_final_newline,
            },
            #[cfg(feature = "napi")]
            FormatFileStrategy::ExternalFormatter { .. } => ResolvedOptions::ExternalFormatter {
                external_options,
                insert_final_newline,
            },
            #[cfg(feature = "napi")]
            FormatFileStrategy::ExternalFormatterPackageJson { .. } => {
                ResolvedOptions::ExternalFormatterPackageJson {
                    external_options,
                    sort_package_json: oxfmt_options.sort_package_json,
                    insert_final_newline,
                }
            }
            #[cfg(not(feature = "napi"))]
            _ => {
                unreachable!("If `napi` feature is disabled, this should not be passed here")
            }
        }
    }
}

// ---

/// JSON formatter options
#[derive(Clone, Debug)]
pub struct JsonFormatterOptions {
    pub indent_width: usize,
    pub use_tabs: bool,
    pub line_ending: String,
    pub trailing_commas: bool,
    pub quote_properties: json5format::QuoteProperties,
}

/// Build JSON formatter options from FormatOptions.
fn build_json_options(format_options: &FormatOptions) -> JsonFormatterOptions {
    JsonFormatterOptions {
        indent_width: format_options.indent_width.value() as usize,
        use_tabs: format_options.indent_style.is_tab(),
        line_ending: if format_options.line_ending.is_carriage_return_line_feed() {
            "\r\n".to_string()
        } else {
            "\n".to_string()
        },
        trailing_commas: format_options.trailing_commas.is_none(),
        quote_properties: match format_options.quote_properties {
            oxc_formatter::QuoteProperties::AsNeeded => json5format::QuoteProperties::AsNeeded,
            oxc_formatter::QuoteProperties::Preserve => json5format::QuoteProperties::Preserve,
            oxc_formatter::QuoteProperties::Consistent => json5format::QuoteProperties::Consistent,
        },
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

    use oxc_formatter::{QuoteProperties, QuoteStyle};

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
        fs::write(&jsonc_path, "{\n  // comment\n  \"singleQuote\": false\n}\n")
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
        let (oxfmt_options, _) = resolver
            .cached_options
            .as_ref()
            .expect("validated config should be cached");

        assert_eq!(ignore_patterns, vec!["dist/**".to_string()]);
        assert_eq!(oxfmt_options.format_options.quote_style, QuoteStyle::Single);
        assert_eq!(
            oxfmt_options.format_options.quote_properties,
            QuoteProperties::Preserve
        );
    }
}
