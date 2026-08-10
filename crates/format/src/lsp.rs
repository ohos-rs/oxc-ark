// LSP integration follows Oxc's oxfmt language server shape while using oxk's
// pure Rust formatter implementation.

use std::{
    fmt::Write,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use oxc_data_structures::rope::{Rope, get_line_column};
use oxc_language_server::{
    Capabilities, LanguageId, TextDocument, Tool, ToolBuilder, ToolRestartChanges, WorkerManager,
    run_server,
};
use oxc_span::ExplicitLanguage;
use serde_json::Value;
use tower_lsp_server::ls_types::{Pattern, Position, Range, ServerCapabilities, TextEdit, Uri};
use tracing::{debug, warn};

use crate::{
    ConfigResolver, FormatFileStrategy, FormatResult, SourceFormatter, build_ignore_matcher,
    is_gitignore_match, resolve_editorconfig_path, resolve_oxfmtrc_path, should_ignore_file,
};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct LspFormatOptions {
    config_path: Option<String>,
    language: Option<ExplicitLanguage>,
}

impl TryFrom<Value> for LspFormatOptions {
    type Error = String;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        if value == Value::Null {
            return Ok(Self::default());
        }

        let Some(object) = value.as_object() else {
            return Err("no object passed".to_string());
        };

        let language = object
            .get("fmt.language")
            .and_then(Value::as_str)
            .map(str::parse::<ExplicitLanguage>)
            .transpose()
            .map_err(|error| error.to_string())?;

        Ok(Self {
            config_path: object
                .get("fmt.configPath")
                .and_then(Value::as_str)
                .map(str::to_owned),
            language,
        })
    }
}

pub async fn run_lsp(
    server_name: String,
    server_version: String,
    language: Option<ExplicitLanguage>,
) {
    run_server(
        server_name,
        server_version_with_vp(server_version),
        WorkerManager::new_dynamic(Arc::new(ServerFormatterBuilder { language })),
    )
    .await;
}

fn server_version_with_vp(mut version: String) -> String {
    if let Some(vp_version) = std::env::var_os("VP_VERSION") {
        let _ = write!(version, " (VP: {})", vp_version.to_string_lossy());
    }
    version
}

struct ServerFormatterBuilder {
    language: Option<ExplicitLanguage>,
}

impl ServerFormatterBuilder {
    fn build(&self, root_uri: &Uri, options: Value) -> ServerFormatter {
        let options = deserialize_lsp_options(options);
        let root_path = root_uri
            .to_file_path()
            .map(std::borrow::Cow::into_owned)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

        let prettierignore_glob = match create_prettierignore_glob(&root_path) {
            Ok(glob) => Some(glob),
            Err(err) => {
                warn!("Failed to create prettierignore globs: {err}");
                None
            }
        };

        ServerFormatter {
            root_path,
            source_formatter: SourceFormatter::new(1),
            prettierignore_glob,
            explicit_config_path: options
                .config_path
                .filter(|path| !path.is_empty())
                .map(PathBuf::from),
            language: options.language.or(self.language),
        }
    }
}

impl ToolBuilder for ServerFormatterBuilder {
    fn server_capabilities(
        &self,
        capabilities: &mut ServerCapabilities,
        _backend_capabilities: &mut Capabilities,
    ) {
        capabilities.document_formatting_provider =
            Some(tower_lsp_server::ls_types::OneOf::Left(true));
    }

    fn build_boxed(&self, root_uri: &Uri, options: Value) -> Box<dyn Tool> {
        Box::new(self.build(root_uri, options))
    }
}

struct ServerFormatter {
    root_path: PathBuf,
    source_formatter: SourceFormatter,
    prettierignore_glob: Option<Gitignore>,
    explicit_config_path: Option<PathBuf>,
    language: Option<ExplicitLanguage>,
}

impl Tool for ServerFormatter {
    fn handle_configuration_change(
        &self,
        builder: &dyn ToolBuilder,
        root_uri: &Uri,
        old_options_json: &Value,
        new_options_json: Value,
    ) -> ToolRestartChanges {
        let old_options = deserialize_lsp_options(old_options_json.clone());
        let new_options = deserialize_lsp_options(new_options_json.clone());
        if old_options == new_options {
            return ToolRestartChanges {
                tool: None,
                watch_patterns: None,
            };
        }

        let new_formatter = builder.build_boxed(root_uri, new_options_json.clone());
        let watch_patterns = new_formatter.get_watcher_patterns(new_options_json);
        ToolRestartChanges {
            tool: Some(new_formatter),
            watch_patterns: Some(watch_patterns),
        }
    }

    fn get_watcher_patterns(&self, options: Value) -> Vec<Pattern> {
        let options = deserialize_lsp_options(options);

        let mut patterns = if let Some(config_path) = options.config_path.filter(|s| !s.is_empty())
        {
            vec![config_path]
        } else {
            vec![
                "**/.oxfmtrc.json".to_string(),
                "**/.oxfmtrc.jsonc".to_string(),
            ]
        };
        patterns.push(".editorconfig".to_string());
        patterns
    }

    fn handle_watched_file_change(
        &self,
        _builder: &dyn ToolBuilder,
        _changed_uri: &Uri,
        _root_uri: &Uri,
        _options: Value,
    ) -> ToolRestartChanges {
        ToolRestartChanges {
            tool: None,
            watch_patterns: None,
        }
    }

    fn run_format(&self, document: &TextDocument) -> Result<Vec<TextEdit>, String> {
        let file_content;
        let (path, source_text) = if document.uri.scheme().as_str() == "file" {
            let path = document
                .uri
                .to_file_path()
                .map(std::borrow::Cow::into_owned)
                .ok_or_else(|| "Invalid file URI".to_string())?;
            let source_text = if let Some(text) = document.text.as_deref() {
                text
            } else {
                file_content = fs::read_to_string(&path)
                    .map_err(|err| format!("Failed to read file {}: {err}", path.display()))?;
                &file_content
            };
            (path, source_text)
        } else {
            let source_text = document
                .text
                .as_deref()
                .ok_or_else(|| "In-memory formatting requires content".to_string())?;
            let Some(path) = create_fake_file_path_from_language_id(
                &document.language_id,
                &self.root_path,
                document.uri,
            ) else {
                return Ok(Vec::new());
            };
            (path, source_text)
        };

        let document_language = get_explicit_language_from_language_id(&document.language_id);
        let Some(result) = self.format_path(&path, source_text, document_language)? else {
            return Ok(Vec::new());
        };

        match result {
            FormatResult::Success { code, is_changed } => {
                if !is_changed {
                    return Ok(Vec::new());
                }

                let (start, end, replacement) = compute_minimal_text_edit(source_text, &code);
                let rope = Rope::from(source_text);
                let (start_line, start_character) = get_line_column(&rope, start, source_text);
                let (end_line, end_character) = get_line_column(&rope, end, source_text);

                Ok(vec![TextEdit::new(
                    Range::new(
                        Position::new(start_line, start_character),
                        Position::new(end_line, end_character),
                    ),
                    replacement.to_string(),
                )])
            }
            FormatResult::Error(_) => Ok(Vec::new()),
        }
    }
}

impl ServerFormatter {
    fn format_path(
        &self,
        path: &Path,
        source_text: &str,
        document_language: Option<ExplicitLanguage>,
    ) -> Result<Option<FormatResult>, String> {
        if should_ignore_file(path) || self.is_prettierignored(path) {
            return Ok(None);
        }

        let Some((resolver, ignore_matcher)) = self.load_config_for_path(path)? else {
            return Ok(None);
        };

        if ignore_matcher
            .as_ref()
            .is_some_and(|matcher| is_gitignore_match(matcher, path, false, true))
        {
            debug!(
                "File is ignored by formatter ignorePatterns: {}",
                path.display()
            );
            return Ok(None);
        }

        let language = document_language.or(self.language);
        let Some(strategy) =
            FormatFileStrategy::from_path_with_language(path.to_path_buf(), language)
        else {
            debug!("Unsupported file type for formatting: {}", path.display());
            return Ok(None);
        };

        if !strategy.can_format_without_external() {
            debug!(
                "Skipping file that requires external formatter support: {}",
                path.display()
            );
            return Ok(None);
        }

        let resolved_options = resolver.resolve(&strategy);
        Ok(Some(self.source_formatter.format(
            &strategy,
            source_text,
            resolved_options,
        )))
    }

    fn load_config_for_path(
        &self,
        path: &Path,
    ) -> Result<Option<(ConfigResolver, Option<Gitignore>)>, String> {
        let cwd = path.parent().unwrap_or(&self.root_path);
        let config_path = if let Some(explicit_config_path) = &self.explicit_config_path {
            Some(
                resolve_oxfmtrc_path(&self.root_path, Some(explicit_config_path)).ok_or_else(
                    || {
                        format!(
                            "Failed to resolve explicit formatter config path {}",
                            explicit_config_path.display()
                        )
                    },
                )?,
            )
        } else {
            resolve_oxfmtrc_path(cwd, None)
        };
        let editorconfig_path =
            resolve_editorconfig_path(cwd).or_else(|| resolve_editorconfig_path(&self.root_path));

        let mut resolver = ConfigResolver::from_config_paths(
            cwd,
            config_path.as_deref(),
            editorconfig_path.as_deref(),
        )?;
        let ignore_patterns = resolver.build_and_validate()?;
        let ignore_root = config_path
            .as_deref()
            .and_then(Path::parent)
            .unwrap_or(&self.root_path);
        let ignore_matcher = build_ignore_matcher(ignore_root, &ignore_patterns)?;

        Ok(Some((resolver, ignore_matcher)))
    }

    fn is_prettierignored(&self, path: &Path) -> bool {
        self.prettierignore_glob.as_ref().is_some_and(|glob| {
            path.starts_with(glob.path())
                && glob
                    .matched_path_or_any_parents(path, path.is_dir())
                    .is_ignore()
        })
    }
}

fn deserialize_lsp_options(value: Value) -> LspFormatOptions {
    match LspFormatOptions::try_from(value) {
        Ok(options) => options,
        Err(err) => {
            warn!("Failed to deserialize formatter LSP options: {err}");
            LspFormatOptions::default()
        }
    }
}

fn create_prettierignore_glob(root_path: &Path) -> Result<Gitignore, String> {
    let mut builder = GitignoreBuilder::new(root_path);
    let path = root_path.join(".prettierignore");
    if path.exists() && builder.add(&path).is_some() {
        return Err(format!("Failed to add ignore file: {}", path.display()));
    }
    builder
        .build()
        .map_err(|_| "Failed to build ignore globs".to_string())
}

fn create_fake_file_path_from_language_id(
    language_id: &LanguageId,
    root: &Path,
    uri: &Uri,
) -> Option<PathBuf> {
    let extension = match language_id.as_str() {
        "javascript" => "js",
        "typescript" => "ts",
        "javascriptreact" => "jsx",
        "typescriptreact" => "tsx",
        "arkts" | "ets" | "ets-static" => "ets",
        "toml" => "toml",
        "json" => "json",
        "jsonc" => "jsonc",
        "json5" => "json5",
        _ => return None,
    };

    let mut name = uri.authority().map_or_else(
        || {
            uri.path()
                .rsplit_once('/')
                .map_or("Untitled", |(_, segment)| segment.as_str())
        },
        |authority| authority.as_str(),
    );
    if name.is_empty() {
        name = "Untitled";
    }

    Some(root.join(format!("{name}.{extension}")))
}

fn get_explicit_language_from_language_id(language_id: &LanguageId) -> Option<ExplicitLanguage> {
    (language_id.as_str() == "ets-static").then_some(ExplicitLanguage::EtsStatic)
}

/// Returns the minimal text edit `(start, end, replacement)` in byte offsets.
#[expect(clippy::cast_possible_truncation)]
fn compute_minimal_text_edit<'a>(
    source_text: &str,
    formatted_text: &'a str,
) -> (u32, u32, &'a str) {
    debug_assert!(source_text != formatted_text);

    let mut prefix_byte = 0;
    for (source_char, formatted_char) in source_text.chars().zip(formatted_text.chars()) {
        if source_char != formatted_char {
            break;
        }
        prefix_byte += source_char.len_utf8();
    }

    let mut suffix_byte = 0;
    let source_bytes = source_text.as_bytes();
    let formatted_bytes = formatted_text.as_bytes();
    let source_len = source_bytes.len();
    let formatted_len = formatted_bytes.len();

    while suffix_byte < source_len - prefix_byte
        && suffix_byte < formatted_len - prefix_byte
        && source_bytes[source_len - 1 - suffix_byte]
            == formatted_bytes[formatted_len - 1 - suffix_byte]
    {
        suffix_byte += 1;
    }

    while suffix_byte > 0
        && (!source_text.is_char_boundary(source_len - suffix_byte)
            || !formatted_text.is_char_boundary(formatted_len - suffix_byte))
    {
        suffix_byte -= 1;
    }

    let start = prefix_byte as u32;
    let end = (source_len - suffix_byte) as u32;
    let replacement_start = prefix_byte;
    let replacement_end = formatted_len - suffix_byte;
    let replacement = &formatted_text[replacement_start..replacement_end];

    (start, end, replacement)
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::Arc,
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    use oxc_language_server::{LanguageId, TextDocument, Tool};
    use serde_json::json;
    use tower_lsp_server::ls_types::Uri;

    use super::ServerFormatterBuilder;

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(prefix: &str) -> Self {
            static NEXT_ID: AtomicU64 = AtomicU64::new(0);

            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be after unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "oxk-format-lsp-{prefix}-{}-{nonce}-{}",
                std::process::id(),
                NEXT_ID.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).expect("test temp dir should be created");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn formats_document_with_lsp_config_path() {
        let dir = TestDir::new("config");
        fs::write(dir.path().join(".oxfmtrc.json"), r#"{"singleQuote": true}"#)
            .expect("config should be written");

        let file_path = dir.path().join("input.ts");
        let uri = Uri::from_file_path(&file_path).expect("file URI should be created");
        let root_uri = Uri::from_file_path(dir.path()).expect("root URI should be created");
        let formatter = ServerFormatterBuilder { language: None }.build(
            &root_uri,
            json!({
                "fmt.configPath": ".oxfmtrc.json"
            }),
        );
        let source: Arc<str> = Arc::from(r#"const message="hello""#);
        let document = TextDocument::new(
            &uri,
            LanguageId::new("typescript".to_string()),
            Some(Arc::clone(&source)),
        );

        let edits = formatter
            .run_format(&document)
            .expect("formatting should succeed");

        assert_eq!(edits.len(), 1);
        assert!(edits[0].new_text.contains("'hello'"));
    }

    #[test]
    fn formats_static_ets_document_language() {
        let dir = TestDir::new("static-ets");
        let file_path = dir.path().join("input.ets");
        let uri = Uri::from_file_path(&file_path).expect("file URI should be created");
        let root_uri = Uri::from_file_path(dir.path()).expect("root URI should be created");
        let formatter = ServerFormatterBuilder { language: None }.build(&root_uri, json!({}));
        let source: Arc<str> = Arc::from("package example.lsp;\nfinal class Box{value:int=1}\n");
        let document = TextDocument::new(
            &uri,
            LanguageId::new("ets-static".to_string()),
            Some(Arc::clone(&source)),
        );

        let edits = formatter
            .run_format(&document)
            .expect("static ETS formatting should succeed");

        assert_eq!(edits.len(), 1);
        assert!(!edits[0].new_text.is_empty());
    }
}
