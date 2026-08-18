use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use ignore::gitignore::Gitignore;
use oxc_data_structures::rope::Rope;
use rustc_hash::{FxHashMap, FxHashSet};
use tower_lsp_server::ls_types::{
    CodeActionContext, CodeActionTriggerKind, DiagnosticOptions, DiagnosticServerCapabilities,
};
use tower_lsp_server::{
    jsonrpc::ErrorCode,
    ls_types::{
        CodeActionKind, CodeActionOptions, CodeActionOrCommand, CodeActionProviderCapability,
        Diagnostic, DiagnosticSeverity, ExecuteCommandOptions, NumberOrString, Pattern, Range,
        ServerCapabilities, Uri, WorkDoneProgressOptions, WorkspaceEdit,
    },
};
use tracing::{debug, error, warn};

use oxc_linter::{
    AllowWarnDeny, Config, ConfigStore, ConfigStoreBuilder, ExternalLinter, ExternalPluginStore,
    FixKind, LINTABLE_EXTENSIONS, LintIgnoreMatcher, LintOptions, LintRunner, LintRunnerBuilder,
    LintServiceOptions, Linter, Oxlintrc, read_to_string,
};

use oxc_language_server::{
    Capabilities, ConcurrentHashMap, DiagnosticMode, DiagnosticResult, TextDocument, Tool,
    ToolBuilder, ToolRestartChanges,
};
use oxc_span::{ExplicitLanguage, SourceType};

use crate::{
    ArktsLintConfig, arkts,
    config_loader::{
        ConfigLoader, build_nested_configs, config_file_names, discover_configs_in_tree,
    },
    lsp::{
        code_actions::{
            CODE_ACTION_KIND_SOURCE_FIX_ALL_DANGEROUS_OXC, CODE_ACTION_KIND_SOURCE_FIX_ALL_OXC,
            apply_all_fix_code_action, apply_dangerous_fix_code_action, apply_fix_code_actions,
            fix_all_text_edit,
        },
        commands::{FIX_ALL_COMMAND_ID, FixAllCommandArgs},
        error_with_position::{
            DiagnosticReport, LinterCodeAction, create_unused_directives_report,
            generate_inverted_diagnostics, message_to_lsp_diagnostic, offset_to_position,
        },
        lsp_file_system::LspFileSystem,
        options::{
            LintOptions as LSPLintOptions, RulesCustomization, Run, UnusedDisableDirectives,
        },
        utils::{normalize_path, range_overlaps},
    },
};

#[derive(Default)]
pub struct ServerLinterBuilder {
    external_linter: Option<ExternalLinter>,
    default_config_path: Option<PathBuf>,
    language: Option<ExplicitLanguage>,
}

impl ServerLinterBuilder {
    pub fn new(
        external_linter: Option<ExternalLinter>,
        default_config_path: Option<PathBuf>,
        language: Option<ExplicitLanguage>,
    ) -> Self {
        Self {
            external_linter,
            default_config_path,
            language,
        }
    }

    /// # Panics
    /// Panics if the root URI cannot be converted to a file path.
    pub fn build(&self, root_uri: &Uri, options: serde_json::Value) -> ServerLinter {
        let options = match serde_json::from_value::<LSPLintOptions>(options) {
            Ok(opts) => opts,
            Err(e) => {
                warn!(
                    "Failed to deserialize LSPLintOptions from JSON: {e}. Falling back to default options."
                );
                LSPLintOptions::default()
            }
        };
        let root_path = root_uri.to_file_path().unwrap();
        let mut external_linter = self.external_linter.as_ref();
        let mut external_plugin_store = ExternalPluginStore::new(external_linter.is_some());

        // Setup JS workspace. This must be done before loading any configs
        if let Some(external_linter) = external_linter {
            let res = (external_linter.create_workspace)(root_uri.as_str().to_string());

            if let Err(err) = res {
                error!("Failed to setup JS workspace:\n{err}\n");
            }
        }

        let config_path = options
            .config_path
            .as_ref()
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
            .or_else(|| self.default_config_path.clone());
        let loader = ConfigLoader::new(
            external_linter,
            &mut external_plugin_store,
            &[],
            Some(root_uri.as_str()),
        );

        let (oxlintrc, arkts_config) = match loader
            .load_root_config_with_arkts_ancestor_search(&root_path, config_path.as_ref())
        {
            Ok(config) => config,
            Err(e) => {
                warn!("Failed to load config: {e}");
                (Oxlintrc::default(), ArktsLintConfig::default())
            }
        };

        let mut nested_ignore_patterns = Vec::new();
        let mut extended_paths = FxHashSet::default();
        let nested_configs = if options.use_nested_configs() {
            self.create_nested_configs(
                &root_path,
                &oxlintrc.path,
                &mut external_plugin_store,
                &mut nested_ignore_patterns,
                &mut extended_paths,
                Some(root_uri.as_str()),
            )
        } else {
            FxHashMap::default()
        };

        let base_patterns = oxlintrc.ignore_patterns.clone();

        let config_builder = match ConfigStoreBuilder::from_oxlintrc(
            false,
            oxlintrc,
            external_linter,
            &mut external_plugin_store,
            Some(root_uri.as_str()),
        ) {
            Ok(builder) => builder,
            Err(e) => {
                warn!("Failed to build config from oxlintrc: {e}");
                ConfigStoreBuilder::default()
            }
        };

        // TODO(refactor): pull this into a shared function, because in oxlint we have the same functionality.
        let use_nested_config = options.use_nested_configs();
        let fix_kind = FixKind::from(options.fix_kind);

        let use_cross_module = config_builder.plugins().has_import()
            || (use_nested_config
                && nested_configs
                    .values()
                    .any(|config| config.plugins().has_import()));

        extended_paths.extend(config_builder.extended_paths.clone());
        let base_config = config_builder
            .build(&mut external_plugin_store)
            .unwrap_or_else(|err| {
                warn!("Failed to build config: {err}");
                ConfigStoreBuilder::empty()
                    .build(&mut ExternalPluginStore::new(false))
                    .unwrap()
            });

        if external_plugin_store.is_empty() {
            external_linter = None;
        }
        let config_store = ConfigStore::new(base_config, nested_configs, external_plugin_store);

        let lint_options = LintOptions {
            fix: fix_kind,
            report_unused_directive: match options.unused_disable_directives {
                Some(UnusedDisableDirectives::Allow) => Some(AllowWarnDeny::Allow),
                Some(UnusedDisableDirectives::Warn) => Some(AllowWarnDeny::Warn),
                Some(UnusedDisableDirectives::Deny) => Some(AllowWarnDeny::Deny),
                None => match config_store.report_unused_disable_directives() {
                    Some(severity) if severity.is_warn_deny() => Some(severity),
                    _ => None,
                },
            },
            ..Default::default()
        };

        let type_aware = options
            .type_aware
            .unwrap_or(config_store.type_aware_enabled());
        let config_store_clone = config_store.clone();

        // Send JS plugins config to JS side
        if let Some(external_linter) = external_linter {
            let res = config_store.external_plugin_store().setup_rule_configs(
                root_path.to_string_lossy().into_owned(),
                Some(root_uri.as_str()),
                external_linter,
            );
            if let Err(err) = res {
                error!("Failed to setup JS plugins config:\n{err}\n");
            }
        }

        let linter = Linter::new(lint_options, config_store, external_linter.cloned())
            .with_workspace_uri(Some(root_uri.as_str()));
        let mut lint_service_options =
            LintServiceOptions::new(root_path.clone()).with_cross_module(use_cross_module);
        let source_type = options
            .language
            .map(super::options::Language::source_type)
            .or_else(|| self.language.map(ExplicitLanguage::source_type));
        if let Some(source_type) = source_type {
            lint_service_options = lint_service_options.with_source_type(source_type);
        }

        if let Some(ts_path) = options.ts_config_path.as_ref() {
            let ts_path = Path::new(ts_path).to_path_buf();
            let ts_path = if ts_path.is_relative() {
                root_path.join(ts_path)
            } else {
                ts_path
            };
            if ts_path.is_file() {
                lint_service_options = lint_service_options.with_tsconfig(&ts_path);
            }
        }

        let runner = match LintRunnerBuilder::new(lint_service_options.clone(), linter)
            .with_type_aware(type_aware)
            .with_fix_kind(fix_kind)
            .build()
        {
            Ok(runner) => runner,
            Err(e) => {
                warn!("Failed to initialize type-aware linting: {e}");
                let linter =
                    Linter::new(lint_options, config_store_clone, external_linter.cloned())
                        .with_workspace_uri(Some(root_uri.as_str()));
                LintRunnerBuilder::new(lint_service_options, linter)
                    .with_type_aware(false)
                    .with_fix_kind(fix_kind)
                    .build()
                    .expect("Failed to build LintRunner without type-aware linting")
            }
        };

        ServerLinter::new(
            options.run,
            root_path.to_path_buf(),
            LintIgnoreMatcher::new(&base_patterns, &root_path, nested_ignore_patterns),
            Self::create_ignore_glob(&root_path),
            extended_paths,
            runner,
            fix_kind,
            lint_options.report_unused_directive,
            options.rules_customization,
            arkts_config,
            config_path,
            source_type,
        )
    }
}

impl ToolBuilder for ServerLinterBuilder {
    fn server_capabilities(
        &self,
        capabilities: &mut ServerCapabilities,
        backend_capabilities: &mut Capabilities,
    ) {
        capabilities.code_action_provider =
            Some(CodeActionProviderCapability::Options(CodeActionOptions {
                code_action_kinds: Some(vec![
                    CodeActionKind::QUICKFIX,
                    CODE_ACTION_KIND_SOURCE_FIX_ALL_OXC,
                    CODE_ACTION_KIND_SOURCE_FIX_ALL_DANGEROUS_OXC,
                    CodeActionKind::SOURCE_FIX_ALL,
                ]),
                work_done_progress_options: WorkDoneProgressOptions::default(),
                resolve_provider: None,
            }));

        capabilities.execute_command_provider = Some(ExecuteCommandOptions {
            commands: vec![FIX_ALL_COMMAND_ID.to_string()],
            work_done_progress_options: WorkDoneProgressOptions::default(),
        });

        // The server supports pull and push diagnostics.
        // Only use push diagnostics if the client does not support pull diagnostics,
        // or we cannot ask the client to refresh diagnostics.
        if !backend_capabilities.pull_diagnostics || !backend_capabilities.refresh_diagnostics {
            backend_capabilities.diagnostic_mode = DiagnosticMode::Push;
        } else {
            backend_capabilities.diagnostic_mode = DiagnosticMode::Pull;
        }

        // tell the client we support pull diagnostics
        capabilities.diagnostic_provider =
            if backend_capabilities.diagnostic_mode == DiagnosticMode::Pull {
                Some(DiagnosticServerCapabilities::Options(
                    DiagnosticOptions::default(),
                ))
            } else {
                None
            };
    }

    fn build_boxed(&self, root_uri: &Uri, options: serde_json::Value) -> Box<dyn Tool> {
        Box::new(self.build(root_uri, options))
    }

    #[expect(unused)]
    fn shutdown(&self, root_uri: &Uri) {
        // We don't currently destroy workspaces.
        // See comment in `destroyWorkspace` in `src-js/workspace/index.ts` for explanation.
        return;

        // Destroy JS workspace
        if let Some(external_linter) = &self.external_linter {
            let res = (external_linter.destroy_workspace)(root_uri.as_str().to_string());

            if let Err(err) = res {
                error!("Failed to destroy JS workspace:\n{err}\n");
            }
        }
    }
}

impl ServerLinterBuilder {
    /// Searches inside root_uri recursively for the default oxlint config files
    /// and insert them inside the nested configuration
    fn create_nested_configs(
        &self,
        root_path: &Path,
        base_config_path: &Path,
        external_plugin_store: &mut ExternalPluginStore,
        nested_ignore_patterns: &mut Vec<(Vec<String>, PathBuf)>,
        extended_paths: &mut FxHashSet<PathBuf>,
        workspace_uri: Option<&str>,
    ) -> FxHashMap<PathBuf, Config> {
        let config_paths = discover_configs_in_tree(root_path, base_config_path);

        let mut loader = ConfigLoader::new(
            self.external_linter.as_ref(),
            external_plugin_store,
            &[],
            workspace_uri,
        );

        let (configs, errors) = loader.load_discovered_with_root_dir(root_path, config_paths);

        for error in errors {
            if let Some(path) = error.path() {
                warn!("Skipping config file {}: {:?}", path.display(), error);
            } else {
                warn!("Skipping config file: {:?}", error);
            }
        }

        build_nested_configs(configs, nested_ignore_patterns, Some(extended_paths))
    }

    #[expect(clippy::filetype_is_file)]
    fn create_ignore_glob(root_path: &Path) -> Vec<Gitignore> {
        let walk = ignore::WalkBuilder::new(root_path)
            .ignore(true)
            .hidden(false)
            .git_global(false)
            .filter_entry(|entry| {
                !(entry.file_name() == ".git"
                    && entry
                        .file_type()
                        .is_some_and(|file_type| file_type.is_dir()))
            })
            .build()
            .flatten();

        let mut gitignore_globs = vec![];
        for entry in walk {
            if !entry.file_type().is_some_and(|v| v.is_file()) {
                continue;
            }
            let ignore_file_path = entry.path();
            if !ignore_file_path
                .file_name()
                .and_then(std::ffi::OsStr::to_str)
                .is_some_and(|v| [".eslintignore", ".gitignore"].contains(&v))
            {
                continue;
            }
            if let Some(ignore_file_dir) = ignore_file_path.parent() {
                let mut builder = ignore::gitignore::GitignoreBuilder::new(ignore_file_dir);
                builder.add(ignore_file_path);
                if let Ok(gitignore) = builder.build() {
                    gitignore_globs.push(gitignore);
                }
            }
        }

        gitignore_globs
    }
}

pub struct ServerLinter {
    run: Run,
    cwd: PathBuf,
    ignore_matcher: LintIgnoreMatcher,
    gitignore_glob: Vec<Gitignore>,
    extended_paths: FxHashSet<PathBuf>,
    code_actions: Arc<ConcurrentHashMap<Uri, Option<Vec<LinterCodeAction>>>>,
    runner: LintRunner,
    fix_kind: FixKind,
    unused_directives_severity: Option<AllowWarnDeny>,
    rules_customization: Option<RulesCustomization>,
    arkts_config: ArktsLintConfig,
    config_path: Option<PathBuf>,
    source_type: Option<SourceType>,
}

impl Tool for ServerLinter {
    /// # Panics
    /// Panics if the root URI cannot be converted to a file path.
    fn handle_configuration_change(
        &self,
        builder: &dyn ToolBuilder,
        root_uri: &Uri,
        old_options_json: &serde_json::Value,
        new_options_json: serde_json::Value,
    ) -> ToolRestartChanges {
        let old_option = match serde_json::from_value::<LSPLintOptions>(old_options_json.clone()) {
            Ok(opts) => opts,
            Err(e) => {
                warn!(
                    "Failed to deserialize LSPLintOptions from JSON: {e}. Falling back to default options."
                );
                LSPLintOptions::default()
            }
        };

        let new_options = match serde_json::from_value::<LSPLintOptions>(new_options_json.clone()) {
            Ok(opts) => opts,
            Err(e) => {
                warn!(
                    "Failed to deserialize LSPLintOptions from JSON: {e}. Falling back to default options."
                );
                LSPLintOptions::default()
            }
        };

        if !Self::needs_restart(&old_option, &new_options) {
            return ToolRestartChanges {
                tool: None,
                watch_patterns: None,
            };
        }

        // get the cached files before refreshing the linter, and revalidate them after
        builder.shutdown(root_uri);
        let new_linter = builder.build_boxed(root_uri, new_options_json.clone());

        let patterns = {
            if old_option.config_path == new_options.config_path
                && old_option.use_nested_configs() == new_options.use_nested_configs()
                && old_option.type_aware == new_options.type_aware
            {
                None
            } else {
                Some(new_linter.get_watcher_patterns(new_options_json))
            }
        };

        ToolRestartChanges {
            tool: Some(new_linter),
            watch_patterns: patterns,
        }
    }

    fn get_watcher_patterns(&self, options: serde_json::Value) -> Vec<Pattern> {
        let options = match serde_json::from_value::<LSPLintOptions>(options) {
            Ok(opts) => opts,
            Err(e) => {
                warn!(
                    "Failed to deserialize LSPLintOptions from JSON: {e}. Falling back to default options."
                );
                LSPLintOptions::default()
            }
        };
        let mut watchers = match options.config_path.as_deref() {
            Some("") | None => match &self.config_path {
                Some(path) => vec![normalize_path(path).to_string_lossy().to_string()],
                None => config_file_names()
                    .into_iter()
                    .map(|name| format!("**/{name}"))
                    .collect(),
            },
            Some(v) => vec![v.to_string()],
        };

        for path in &self.extended_paths {
            // Ignore known config files when nested config discovery handles them.
            if config_file_names().iter().any(|name| path.ends_with(name))
                && options.use_nested_configs()
            {
                continue;
            }

            let pattern = path.strip_prefix(self.cwd.clone()).unwrap_or(path);

            watchers.push(normalize_path(pattern).to_string_lossy().to_string());
        }

        if options.type_aware.unwrap_or(self.runner.has_type_aware()) {
            watchers.push("**/tsconfig*.json".to_string());
        }

        watchers
    }

    fn handle_watched_file_change(
        &self,
        builder: &dyn ToolBuilder,
        _changed_uri: &Uri,
        root_uri: &Uri,
        options: serde_json::Value,
    ) -> ToolRestartChanges {
        // TODO: Check if the changed file is actually a config file (including extended paths)
        builder.shutdown(root_uri);
        let new_linter = builder.build_boxed(root_uri, options);

        ToolRestartChanges {
            tool: Some(new_linter),
            // TODO: update watch patterns if config_path changed, or the extended paths changed
            watch_patterns: None,
        }
    }

    /// Tries to execute the given command with the provided arguments.
    /// If the command is not recognized, returns `Err(ErrorCode)`.
    /// If the command is recognized and executed it can return:
    /// - `Ok(Some(WorkspaceEdit))` if the command was executed successfully and produced a workspace edit.
    /// - `Ok(None)` if the command was executed successfully but did not produce any workspace edit.
    ///
    /// # Errors
    /// Returns an `ErrorCode::InvalidParams` if the command arguments are invalid.
    fn execute_command(
        &self,
        command: &str,
        arguments: Vec<serde_json::Value>,
    ) -> Result<Option<WorkspaceEdit>, ErrorCode> {
        if command != FIX_ALL_COMMAND_ID {
            return Err(ErrorCode::InvalidParams);
        }

        let args = FixAllCommandArgs::try_from(arguments).map_err(|_| ErrorCode::InvalidParams)?;
        let uri: Uri = args.uri.parse().map_err(|_| ErrorCode::InvalidParams)?;

        if !self.is_responsible_for_uri(&uri) {
            return Ok(None);
        }

        let actions = self.get_code_actions_for_uri(&uri, Some(CodeActionTriggerKind::INVOKED));

        let Some(actions) = actions else {
            return Ok(None);
        };

        if actions.is_empty() {
            return Ok(None);
        }

        let text_edits = fix_all_text_edit(actions.into_iter());

        Ok(Some(WorkspaceEdit {
            #[allow(clippy::disallowed_types)]
            changes: Some(std::collections::HashMap::from([(uri, text_edits)])),
            document_changes: None,
            change_annotations: None,
        }))
    }

    fn get_code_actions_or_commands(
        &self,
        uri: &Uri,
        range: &Range,
        context: &CodeActionContext,
    ) -> Vec<CodeActionOrCommand> {
        let actions = self.get_code_actions_for_uri(uri, context.trigger_kind);

        let Some(actions) = actions else {
            return vec![];
        };

        if actions.is_empty() {
            return vec![];
        }

        let actions = actions
            .into_iter()
            .filter(|r| range_overlaps(*range, r.range));

        // `context.only` is a special case here. ESLint behavior is if `source.fixAll` is the first element in `context.only`,
        // then only return fix all code action, and ignore other code actions, even if they are requested.
        // https://github.com/microsoft/vscode-eslint/blob/1572a25c619861a812c6593c9b130ee52361bcf0/server/src/eslintServer.ts#L587-L589
        // This works for zed editor too, it sends always with this layout: `"only": ["quickfix", "source.fixAll.oxc", "source.fixAll"]`
        // https://github.com/oxc-project/oxc-zed/issues/133#issuecomment-4007046920
        // To align more with the official LSP specs, we implement it a bit differently:
        // If no `context.only` is applied, only return the quick fix code actions.
        // If it is provided, we should loop over it and return the actions in the same order.
        // https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#codeActionContext
        let applying_kinds = match &context.only {
            Some(only) => {
                // `source.fixAll` and `source.fixAll.oxc` should behave the same, filter duplicate out
                let mut seen = FxHashSet::default();
                only.iter()
                    .filter_map(|kind| {
                        if kind == &CODE_ACTION_KIND_SOURCE_FIX_ALL_OXC
                            || kind == &CodeActionKind::SOURCE_FIX_ALL
                        {
                            if seen.contains(&CodeActionKind::SOURCE_FIX_ALL) {
                                None
                            } else {
                                seen.insert(CodeActionKind::SOURCE_FIX_ALL);
                                Some(CodeActionKind::SOURCE_FIX_ALL)
                            }
                        } else {
                            Some(kind.clone())
                        }
                    })
                    .collect::<Vec<_>>()
            }
            // if `only` is not provided, only return quickfixes
            None => vec![CodeActionKind::QUICKFIX],
        };

        let mut code_actions_vec: Vec<CodeActionOrCommand> = vec![];

        for kind in applying_kinds {
            // `CODE_ACTION_KIND_SOURCE_FIX_ALL_OXC` was filtered out by `applying_kinds`, so we don't need to check it here.
            if kind == CodeActionKind::SOURCE_FIX_ALL {
                let Some(fix_all) = apply_all_fix_code_action(
                    actions.clone(),
                    uri.clone(),
                    self.rules_customization.as_ref(),
                ) else {
                    continue;
                };
                code_actions_vec.push(CodeActionOrCommand::CodeAction(fix_all));
            } else if kind == CODE_ACTION_KIND_SOURCE_FIX_ALL_DANGEROUS_OXC {
                if !self.fix_kind.is_dangerous() {
                    warn!(
                        "Linter is not configured to provide dangerous fixes. Please set `fixKind` to `dangerous_fix` or `dangerous_fix_or_suggestion` in the server configuration to enable it."
                    );
                    continue;
                }
                let Some(fix_all) = apply_dangerous_fix_code_action(
                    actions.clone(),
                    uri.clone(),
                    self.rules_customization.as_ref(),
                ) else {
                    continue;
                };
                code_actions_vec.push(CodeActionOrCommand::CodeAction(fix_all));
            } else if kind == CodeActionKind::QUICKFIX {
                for action in actions.clone() {
                    let fix_actions = apply_fix_code_actions(action, uri);
                    code_actions_vec
                        .extend(fix_actions.into_iter().map(CodeActionOrCommand::CodeAction));
                }
            }
        }

        code_actions_vec
    }

    /// Lint a file with the current linter
    /// - If the file is not lintable or ignored, an empty vector is returned
    fn run_diagnostic(&self, document: &TextDocument) -> DiagnosticResult {
        Ok(vec![(
            document.uri.clone(),
            self.run_file(document.uri, document.text.as_deref())?,
        )])
    }

    /// Lint a file with the current linter
    /// - If the file is not lintable or ignored, an empty vector is returned
    /// - If the linter is not set to `OnType`, an empty vector is returned
    fn run_diagnostic_on_change(&self, document: &TextDocument) -> DiagnosticResult {
        if self.run != Run::OnType {
            return Ok(vec![]);
        }
        self.run_diagnostic(document)
    }

    /// Lint a file with the current linter
    /// - If the file is not lintable or ignored, an empty vector is returned
    /// - If the linter is not set to `OnSave`, an empty vector is returned
    fn run_diagnostic_on_save(&self, document: &TextDocument) -> DiagnosticResult {
        if self.run != Run::OnSave {
            return Ok(vec![]);
        }
        self.run_diagnostic(document)
    }

    fn remove_uri_cache(&self, uri: &Uri) {
        self.code_actions.pin().remove(uri);
    }
}

impl ServerLinter {
    /// # Panics
    /// Panics if the root URI cannot be converted to a file path.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        run: Run,
        cwd: PathBuf,
        ignore_matcher: LintIgnoreMatcher,
        gitignore_glob: Vec<Gitignore>,
        extended_paths: FxHashSet<PathBuf>,
        runner: LintRunner,
        fix_kind: FixKind,
        unused_directives_severity: Option<AllowWarnDeny>,
        rules_customization: Option<RulesCustomization>,
        arkts_config: ArktsLintConfig,
        config_path: Option<PathBuf>,
        source_type: Option<SourceType>,
    ) -> Self {
        Self {
            run,
            cwd,
            ignore_matcher,
            gitignore_glob,
            extended_paths,
            code_actions: Arc::new(ConcurrentHashMap::default()),
            runner,
            fix_kind,
            unused_directives_severity,
            rules_customization,
            arkts_config,
            config_path,
            source_type,
        }
    }

    fn get_code_actions_for_uri(
        &self,
        uri: &Uri,
        trigger_kind: Option<CodeActionTriggerKind>,
    ) -> Option<Vec<LinterCodeAction>> {
        if let Some(cached_code_actions) = self.code_actions.pin().get(uri) {
            cached_code_actions.clone()
        }
        // only run linting and generate code actions when the code action is explicitly invoked,
        // otherwise it will be too heavy to run linting on every file open or cursor move, which will cause performance issues and a bad user experience.
        // It is most likely that the client already sent a request, where we run the lint process and cache the code actions.
        else if trigger_kind == Some(CodeActionTriggerKind::INVOKED) {
            let _ = self.run_file(uri, None);
            self.code_actions
                .pin()
                .get(uri)
                .and_then(std::clone::Clone::clone)
        } else {
            None
        }
    }

    fn is_lintable_extension(path: &Path) -> bool {
        static WANTED_EXTENSIONS: OnceLock<FxHashSet<&'static str>> = OnceLock::new();
        let wanted_exts =
            WANTED_EXTENSIONS.get_or_init(|| LINTABLE_EXTENSIONS.iter().copied().collect());

        path.extension()
            .and_then(std::ffi::OsStr::to_str)
            .is_some_and(|ext| wanted_exts.contains(ext))
    }

    fn is_ignored(&self, uri_path: &Path) -> bool {
        if !Self::is_lintable_extension(uri_path) {
            debug!("ignored (unsupported extension): {uri_path:?}");
            return true;
        }

        if self.ignore_matcher.should_ignore(uri_path) {
            debug!("ignored: {uri_path:?}");
            return true;
        }

        for gitignore in &self.gitignore_glob {
            if !uri_path.starts_with(gitignore.path()) {
                continue;
            }
            if gitignore
                .matched_path_or_any_parents(uri_path, uri_path.is_dir())
                .is_ignore()
            {
                debug!("ignored: {uri_path:?}");
                return true;
            }
        }
        false
    }

    /// Lint a single file, returning an empty diagnostics list if the file is ignored.
    fn run_file(&self, uri: &Uri, content: Option<&str>) -> Result<Vec<Diagnostic>, String> {
        let Some(uri_path) = uri.to_file_path() else {
            return Ok(Vec::new());
        };
        if self.is_ignored(&uri_path) {
            return Ok(Vec::new());
        }

        let reports = self.lint_path(&uri_path, uri, content)?;

        let mut diagnostics = Vec::with_capacity(reports.len());
        // mostly all diagnostics will have code actions (fix + ignoring line/file), only following diagnostics won't:
        // - inverted diagnostics (related spans for the diagnostics)
        // - diagnostics with span(0,0) and no fixes
        // - tsgolint internal diagnostics
        // - unused directives diagnostics
        let mut code_actions = vec![];
        for report in reports {
            diagnostics.push(report.diagnostic);

            if let Some(code_action) = report.code_action {
                code_actions.push(code_action);
            }
        }

        self.code_actions
            .pin()
            .insert(uri.clone(), Some(code_actions));

        Ok(diagnostics)
    }

    fn lint_path(
        &self,
        path: &Path,
        uri: &Uri,
        content: Option<&str>,
    ) -> Result<Vec<DiagnosticReport>, String> {
        debug!("lint {}", path.display());

        let source_text = if let Some(content) = content {
            content
        } else {
            &read_to_string(path).map_err(|e| format!("Failed to read file: {e}"))?
        };

        let rope = &Rope::from_str(source_text);

        let mut fs = LspFileSystem::default();
        fs.add_file(path.to_path_buf(), Arc::from(source_text));

        let mut messages: Vec<DiagnosticReport> =
            match self.runner.run_source(&[Arc::from(path.as_os_str())], &fs) {
                Ok(results) => results
                    .into_iter()
                    .filter_map(|message| {
                        message_to_lsp_diagnostic(
                            message,
                            uri,
                            source_text,
                            rope,
                            self.rules_customization.as_ref(),
                        )
                    })
                    .collect(),
                Err(e) => {
                    // clear disable directives on error to prevent stale directives
                    self.runner.directives_coordinator().remove(path);
                    return Err(e);
                }
            };

        messages.append(&mut generate_inverted_diagnostics(&messages, uri));
        messages.extend(self.lint_arkts_path(path, source_text, rope)?);

        // Add unused directives if configured
        if let Some(severity) = self.unused_directives_severity
            && let Some(directives) = self.runner.directives_coordinator().get(path)
        {
            messages.extend(create_unused_directives_report(
                &directives,
                severity,
                source_text,
                rope,
            ));
        }

        // Clear any stale directives because they are no longer needed.
        // This prevents using outdated directive spans if the new linting run fails.
        self.runner.directives_coordinator().remove(path);

        Ok(messages)
    }

    fn lint_arkts_path(
        &self,
        path: &Path,
        source_text: &str,
        rope: &Rope,
    ) -> Result<Vec<DiagnosticReport>, String> {
        if self.arkts_config.is_empty()
            || path.extension().and_then(std::ffi::OsStr::to_str) != Some("ets")
        {
            return Ok(Vec::new());
        }

        let diagnostics = arkts::lint_standalone_source(
            path,
            source_text,
            &self.arkts_config.rules,
            &self.cwd,
            self.source_type,
        )?;

        Ok(diagnostics
            .into_iter()
            .map(|diagnostic| arkts_diagnostic_to_lsp(diagnostic, source_text, rope))
            .collect())
    }

    fn needs_restart(old_options: &LSPLintOptions, new_options: &LSPLintOptions) -> bool {
        old_options.config_path != new_options.config_path
            || old_options.ts_config_path != new_options.ts_config_path
            || old_options.use_nested_configs() != new_options.use_nested_configs()
            || old_options.fix_kind != new_options.fix_kind
            || old_options.unused_disable_directives != new_options.unused_disable_directives
            || old_options.language != new_options.language
            // TODO: only the TsgoLinter needs to be dropped or created
            || old_options.type_aware != new_options.type_aware
    }

    /// Check if the linter is responsible for the given URI.
    /// e.g. root URI: file:///path/to/root
    ///      responsible for: file:///path/to/root/file.js
    ///      not responsible for: file:///path/to/other/file.js
    fn is_responsible_for_uri(&self, uri: &Uri) -> bool {
        if let Some(path) = uri.to_file_path() {
            return path.starts_with(&self.cwd);
        }
        false
    }
}

fn arkts_diagnostic_to_lsp(
    diagnostic: arkts::StandaloneDiagnostic,
    source_text: &str,
    rope: &Rope,
) -> DiagnosticReport {
    let range = Range::new(
        offset_to_position(rope, diagnostic.start, source_text),
        offset_to_position(rope, diagnostic.end, source_text),
    );
    let severity = match diagnostic.severity {
        arkts::StandaloneSeverity::Warn => DiagnosticSeverity::WARNING,
        arkts::StandaloneSeverity::Error => DiagnosticSeverity::ERROR,
    };

    DiagnosticReport {
        diagnostic: Diagnostic {
            range,
            severity: Some(severity),
            code: Some(NumberOrString::String(diagnostic.rule_name)),
            message: diagnostic.message,
            source: Some("arkts".to_string()),
            code_description: None,
            related_information: None,
            tags: None,
            data: None,
        },
        code_action: None,
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, sync::Arc};

    use oxc_language_server::{Capabilities, LanguageId, TextDocument, Tool, ToolBuilder};
    use serde_json::json;
    use tower_lsp_server::ls_types::{
        CodeActionContext, CodeActionKind, CodeActionOrCommand, CodeActionProviderCapability,
        CodeActionTriggerKind, Position, Range, ServerCapabilities, Uri,
    };

    use crate::{
        TempArktsConfig,
        lsp::{
            code_actions::{
                CODE_ACTION_KIND_SOURCE_FIX_ALL_DANGEROUS_OXC, CODE_ACTION_KIND_SOURCE_FIX_ALL_OXC,
            },
            commands::FIX_ALL_COMMAND_ID,
        },
    };

    use super::ServerLinterBuilder;

    #[test]
    fn server_capabilities_include_quickfix_and_fix_all() {
        let builder = ServerLinterBuilder::default();
        let mut server_capabilities = ServerCapabilities::default();
        let mut backend_capabilities = Capabilities::default();

        builder.server_capabilities(&mut server_capabilities, &mut backend_capabilities);

        match &server_capabilities.code_action_provider {
            Some(CodeActionProviderCapability::Options(options)) => {
                let kinds = options.code_action_kinds.as_ref().unwrap();
                assert!(kinds.contains(&CodeActionKind::QUICKFIX));
                assert!(kinds.contains(&CODE_ACTION_KIND_SOURCE_FIX_ALL_OXC));
                assert!(kinds.contains(&CODE_ACTION_KIND_SOURCE_FIX_ALL_DANGEROUS_OXC));
                assert!(kinds.contains(&CodeActionKind::SOURCE_FIX_ALL));
            }
            _ => panic!("expected code action provider options"),
        }

        let execute_command_provider = server_capabilities
            .execute_command_provider
            .as_ref()
            .unwrap();
        assert!(
            execute_command_provider
                .commands
                .contains(&FIX_ALL_COMMAND_ID.to_string())
        );
    }

    #[test]
    fn diagnostics_include_arkts_rules_while_oxlint_quickfix_stays_available() {
        let temp = TempArktsConfig::new().unwrap();
        let root = temp.config_path().parent().unwrap().to_path_buf();
        let config_path = root.join(".oxlintrc.json");
        fs::write(
            &config_path,
            r#"{
  "plugins": ["arkts"],
  "rules": {
    "no-console": "error",
    "arkts/no-symbol": "error"
  }
}
"#,
        )
        .unwrap();

        let source = r#"console.log(Symbol("id"));
"#;
        let file_path = root.join("input.ets");
        fs::write(&file_path, source).unwrap();

        let root_uri = Uri::from_file_path(&root).unwrap();
        let file_uri = Uri::from_file_path(&file_path).unwrap();
        let linter = ServerLinterBuilder::new(None, None, None).build(&root_uri, json!({}));
        let document = TextDocument::new(
            &file_uri,
            LanguageId::new("typescript".to_string()),
            Some(Arc::<str>::from(source)),
        );

        let result = linter.run_diagnostic(&document).unwrap();
        let diagnostics = &result[0].1;

        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.source.as_deref() == Some("arkts")
                    && diagnostic.code.as_ref().is_some_and(|code| matches!(
                        code,
                        tower_lsp_server::ls_types::NumberOrString::String(code)
                            if code == "no-symbol"
                    ))),
            "expected ArkTS no-symbol diagnostic, got {diagnostics:#?}"
        );
        assert!(
            diagnostics
                .iter()
                .any(
                    |diagnostic| diagnostic.code.as_ref().is_some_and(|code| matches!(
                        code,
                        tower_lsp_server::ls_types::NumberOrString::String(code)
                            if code.contains("no-console")
                    ))
                ),
            "expected oxlint no-console diagnostic, got {diagnostics:#?}"
        );

        let context = CodeActionContext {
            diagnostics: diagnostics.clone(),
            only: None,
            trigger_kind: Some(CodeActionTriggerKind::INVOKED),
        };
        let actions = linter.get_code_actions_or_commands(
            &file_uri,
            &Range::new(Position::new(0, 0), Position::new(0, 26)),
            &context,
        );

        assert!(
            actions.iter().any(|action| match action {
                CodeActionOrCommand::CodeAction(action) =>
                    action.title.contains("Disable no-console"),
                CodeActionOrCommand::Command(_) => false,
            }),
            "expected no-console quickfix, got {actions:#?}"
        );
    }
}
