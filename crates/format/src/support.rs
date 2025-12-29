use std::path::{Path, PathBuf};

use phf::phf_set;

use oxc_formatter::get_supported_source_type;
use oxc_span::SourceType;

#[derive(Debug)]
pub enum FormatFileStrategy {
    OxcFormatter {
        path: PathBuf,
        source_type: SourceType,
    },
    /// TOML files formatted by oxc_toml (Pure Rust).
    OxfmtToml { path: PathBuf },
    /// JSON/JSON5/JSONC files formatted by Rust formatter (Pure Rust).
    OxfmtJson { path: PathBuf, json_type: JsonType },
    ExternalFormatter {
        path: PathBuf,
        #[cfg_attr(not(feature = "napi"), expect(dead_code))]
        parser_name: &'static str,
    },
    /// `package.json` is special: sorted by `sort-package-json` then formatted by external formatter.
    ExternalFormatterPackageJson {
        path: PathBuf,
        #[cfg_attr(not(feature = "napi"), expect(dead_code))]
        parser_name: &'static str,
    },
}

/// JSON file type for formatting
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JsonType {
    /// Standard JSON
    Json,
    /// JSON5 (supports comments, trailing commas, etc.)
    Json5,
    /// JSONC (JSON with comments)
    Jsonc,
}

impl TryFrom<PathBuf> for FormatFileStrategy {
    type Error = ();

    fn try_from(path: PathBuf) -> Result<Self, Self::Error> {
        // Check JS/TS files first
        if let Some(source_type) = get_supported_source_type(&path) {
            return Ok(Self::OxcFormatter { path, source_type });
        }

        // Extract file_name and extension once for all subsequent checks
        let Some(file_name) = path.file_name().and_then(|f| f.to_str()) else {
            return Err(());
        };

        // Excluded files like lock files
        if EXCLUDE_FILENAMES.contains(file_name) {
            return Err(());
        }

        // Then TOML files
        if is_toml_file(file_name) {
            return Ok(Self::OxfmtToml { path });
        }

        // Then JSON/JSON5/JSONC files (before external formatter)
        let extension = path.extension().and_then(|ext| ext.to_str());
        if let Some(json_type) = get_json_type(file_name, extension) {
            return Ok(Self::OxfmtJson { path, json_type });
        }

        // Then external formatter files
        // `package.json` is special: sorted then formatted
        if file_name == "package.json" {
            return Ok(Self::ExternalFormatterPackageJson {
                path,
                parser_name: "json-stringify",
            });
        }

        if let Some(parser_name) = get_external_parser_name(file_name, extension) {
            return Ok(Self::ExternalFormatter { path, parser_name });
        }

        Err(())
    }
}

impl FormatFileStrategy {
    #[cfg(not(feature = "napi"))]
    pub fn can_format_without_external(&self) -> bool {
        matches!(
            self,
            Self::OxcFormatter { .. } | Self::OxfmtToml { .. } | Self::OxfmtJson { .. }
        )
    }

    pub fn path(&self) -> &Path {
        match self {
            Self::OxcFormatter { path, .. }
            | Self::OxfmtToml { path }
            | Self::OxfmtJson { path, .. }
            | Self::ExternalFormatter { path, .. }
            | Self::ExternalFormatterPackageJson { path, .. } => path,
        }
    }
}

static EXCLUDE_FILENAMES: phf::Set<&'static str> = phf_set! {
    // JSON, YAML lock files
    "package-lock.json",
    "pnpm-lock.yaml",
    "yarn.lock",
    "MODULE.bazel.lock",
    "bun.lock",
    "deno.lock",
    "composer.lock",
    "Package.resolved",
    "Pipfile.lock",
    "flake.lock",
    "mcmod.info",
    // TOML lock files
    "Cargo.lock",
    "Gopkg.lock",
    "pdm.lock",
    "poetry.lock",
    "uv.lock",
};

// ---

/// Returns `true` if this is a TOML file.
fn is_toml_file(file_name: &str) -> bool {
    if TOML_FILENAMES.contains(file_name) {
        return true;
    }

    #[expect(clippy::case_sensitive_file_extension_comparisons)]
    if file_name.ends_with(".toml.example") || file_name.ends_with(".toml") {
        return true;
    }

    false
}

static TOML_FILENAMES: phf::Set<&'static str> = phf_set! {
    "Pipfile",
    "Cargo.toml.orig",
};

// ---

/// Returns JSON type for JSON/JSON5/JSONC files.
/// Returns `None` if this is not a JSON file, or if it should be handled by external formatter (e.g., package.json).
fn get_json_type(file_name: &str, extension: Option<&str>) -> Option<JsonType> {
    // Skip package.json - it's handled separately by ExternalFormatterPackageJson
    if file_name == "package.json" {
        return None;
    }

    // Check JSON5 files first (by extension)
    if extension == Some("json5") {
        return Some(JsonType::Json5);
    }

    // Check JSONC files (by extension)
    if let Some(ext) = extension {
        if JSONC_EXTENSIONS.contains(ext) {
            return Some(JsonType::Jsonc);
        }
    }

    // Check standard JSON files (by extension)
    if let Some(ext) = extension {
        if JSON_EXTENSIONS.contains(ext) {
            return Some(JsonType::Json);
        }
    }

    // Check JSON filenames
    if JSON_FILENAMES.contains(file_name) {
        return Some(JsonType::Json);
    }

    None
}

// ---

/// Returns parser name for external formatter, if supported.
/// See also `prettier --support-info | jq '.languages[]'`
fn get_external_parser_name(file_name: &str, extension: Option<&str>) -> Option<&'static str> {
    // JSON and variants
    // NOTE: `package.json` is handled separately in `FormatFileStrategy::try_from()`
    if file_name == "composer.json" || extension == Some("importmap") {
        return Some("json-stringify");
    }
    if JSON_FILENAMES.contains(file_name) {
        return Some("json");
    }
    if let Some(ext) = extension {
        if JSON_EXTENSIONS.contains(ext) {
            return Some("json");
        }
        if JSONC_EXTENSIONS.contains(ext) {
            return Some("jsonc");
        }
    }
    if extension == Some("json5") {
        return Some("json5");
    }

    // YAML
    if YAML_FILENAMES.contains(file_name) {
        return Some("yaml");
    }
    if let Some(ext) = extension
        && YAML_EXTENSIONS.contains(ext)
    {
        return Some("yaml");
    }

    // Markdown and variants
    if MARKDOWN_FILENAMES.contains(file_name) {
        return Some("markdown");
    }
    if let Some(ext) = extension
        && MARKDOWN_EXTENSIONS.contains(ext)
    {
        return Some("markdown");
    }
    if extension == Some("mdx") {
        return Some("mdx");
    }

    // HTML and variants
    // Must be checked before generic HTML
    if file_name.ends_with(".component.html") {
        return Some("angular");
    }
    if let Some(ext) = extension
        && HTML_EXTENSIONS.contains(ext)
    {
        return Some("html");
    }
    if extension == Some("vue") {
        return Some("vue");
    }
    if extension == Some("mjml") {
        return Some("mjml");
    }

    // CSS and variants
    if let Some(ext) = extension
        && CSS_EXTENSIONS.contains(ext)
    {
        return Some("css");
    }
    if extension == Some("less") {
        return Some("less");
    }
    if extension == Some("scss") {
        return Some("scss");
    }

    // GraphQL
    if let Some(ext) = extension {
        if GRAPHQL_EXTENSIONS.contains(ext) {
            return Some("graphql");
        }
        if HANDLEBARS_EXTENSIONS.contains(ext) {
            return Some("glimmer");
        }
    }

    None
}

static JSON_EXTENSIONS: phf::Set<&'static str> = phf_set! {
    "json",
    "4DForm",
    "4DProject",
    "avsc",
    "geojson",
    "gltf",
    "har",
    "ice",
    "JSON-tmLanguage",
    "json.example",
    "mcmeta",
    "sarif",
    "tact",
    "tfstate",
    "tfstate.backup",
    "topojson",
    "webapp",
    "webmanifest",
    "yy",
    "yyp",
};

static JSON_FILENAMES: phf::Set<&'static str> = phf_set! {
    ".all-contributorsrc",
    ".arcconfig",
    ".auto-changelog",
    ".c8rc",
    ".htmlhintrc",
    ".imgbotconfig",
    ".nycrc",
    ".tern-config",
    ".tern-project",
    ".watchmanconfig",
    ".babelrc",
    ".jscsrc",
    ".jshintrc",
    ".jslintrc",
    ".swcrc",
};

static JSONC_EXTENSIONS: phf::Set<&'static str> = phf_set! {
    "jsonc",
    "code-snippets",
    "code-workspace",
    "sublime-build",
    "sublime-color-scheme",
    "sublime-commands",
    "sublime-completions",
    "sublime-keymap",
    "sublime-macro",
    "sublime-menu",
    "sublime-mousemap",
    "sublime-project",
    "sublime-settings",
    "sublime-theme",
    "sublime-workspace",
    "sublime_metrics",
    "sublime_session",
};

static HTML_EXTENSIONS: phf::Set<&'static str> = phf_set! {
    "html",
    "hta",
    "htm",
    "inc",
    "xht",
    "xhtml",
};

static CSS_EXTENSIONS: phf::Set<&'static str> = phf_set! {
    "css",
    "wxss",
    "pcss",
    "postcss",
};

static GRAPHQL_EXTENSIONS: phf::Set<&'static str> = phf_set! {
    "graphql",
    "gql",
    "graphqls",
};

static HANDLEBARS_EXTENSIONS: phf::Set<&'static str> = phf_set! {
    "handlebars",
    "hbs",
};

static MARKDOWN_FILENAMES: phf::Set<&'static str> = phf_set! {
    "contents.lr",
    "README",
};

static MARKDOWN_EXTENSIONS: phf::Set<&'static str> = phf_set! {
    "md",
    "livemd",
    "markdown",
    "mdown",
    "mdwn",
    "mkd",
    "mkdn",
    "mkdown",
    "ronn",
    "scd",
    "workbook",
};

static YAML_FILENAMES: phf::Set<&'static str> = phf_set! {
    ".clang-format",
    ".clang-tidy",
    ".clangd",
    ".gemrc",
    "CITATION.cff",
    "glide.lock",
    "pixi.lock",
    ".prettierrc",
    ".stylelintrc",
    ".lintstagedrc",
};

static YAML_EXTENSIONS: phf::Set<&'static str> = phf_set! {
    "yml",
    "mir",
    "reek",
    "rviz",
    "sublime-syntax",
    "syntax",
    "yaml",
    "yaml-tmlanguage",
};
