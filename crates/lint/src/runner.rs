// Derived from ohos-rs/oxc 230deff13c3d0baddb9560ba895ee2b6a3fc4b7e (MIT).
// Copyright (c) Oxc project contributors.

use crate::{ArktsLintConfig, arkts};
use oxc_span::{SourceType, Span};
use oxlint::cli::{
    CliRunResult, DebugOption, LintCommand, MiscOptions, ReportUnusedDirectives, WarningOptions,
};
use std::fs;
use std::{
    env,
    ffi::OsStr,
    fmt::Debug,
    io::{ErrorKind, Write},
    path::{Path, PathBuf, absolute},
    sync::Arc,
    time::Instant,
};

use cow_utils::CowUtils;
use ignore::{gitignore::Gitignore, overrides::OverrideBuilder};

use oxc_config::GitignoreChecker;
use oxc_diagnostics::{
    DiagnosticSender, DiagnosticService, GraphicalReportHandler, GraphicalTheme, OxcDiagnostic,
};
use oxc_linter::{
    AllowWarnDeny, ConfigBuilderError, ConfigStore, ConfigStoreBuilder, ExternalLinter,
    ExternalPluginStore, InvalidFilterKind, LintFilter, LintOptions, LintRunner,
    LintServiceOptions, Linter, OxlintSuppressionFileAction, RuleTimingStore, SuppressionManager,
};

use crate::{
    config_loader::{
        CliConfigLoadError, ConfigLoadError, ConfigLoader, config_discovery,
        materialize_default_plugins,
    },
    output_formatter::{LintCommandInfo, OutputFormatter},
    walk::Walk,
};
use oxc_linter::LintIgnoreMatcher;

pub struct OxkLintRunner {
    options: LintCommand,
    cwd: PathBuf,
    external_linter: Option<ExternalLinter>,
    arkts_config: crate::ArktsLintConfig,
}

impl Debug for OxkLintRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut s = f.debug_struct("OxkLintRunner");
        s.field("options", &self.options)
            .field("cwd", &self.cwd)
            .field(
                "external_linter",
                if self.external_linter.is_some() {
                    &"Some(ExternalLinter)"
                } else {
                    &"None"
                },
            );
        s.finish()
    }
}

impl OxkLintRunner {
    /// # Panics
    pub fn new(
        options: LintCommand,
        arkts_config: crate::ArktsLintConfig,
        external_linter: Option<ExternalLinter>,
    ) -> Self {
        Self {
            options,
            arkts_config,
            cwd: env::current_dir().expect("Failed to get current working directory"),
            external_linter,
        }
    }

    /// # Panics
    pub fn run(self, stdout: &mut dyn Write) -> CliRunResult {
        let format_str = self.options.output_options.format;
        let debug_files = self
            .options
            .output_options
            .debug
            .contains(DebugOption::Files);
        let debug_timings = self
            .options
            .output_options
            .debug
            .contains(DebugOption::Timings);
        let output_formatter = OutputFormatter::from_debug_name(&format!("{format_str:?}"));

        let LintCommand {
            paths,
            filter,
            basic_options,
            warning_options,
            ignore_options,
            fix_options,
            enable_plugins,
            misc_options,
            disable_nested_config,
            inline_config_options,
            suppression_options,
            lang,
            ..
        } = self.options;

        if basic_options.init {
            return crate::mode::run_init(&self.cwd, stdout);
        }

        let external_linter = self.external_linter.as_ref();

        let mut paths = paths;
        let provided_path_count = paths.len();
        let now = Instant::now();

        let filters = match Self::get_filters(filter) {
            Ok(filters) => filters,
            Err((result, message)) => {
                print_and_flush_stdout(stdout, &message);
                return result;
            }
        };

        let handler = if cfg!(any(test, feature = "testing")) {
            GraphicalReportHandler::new_themed(GraphicalTheme::none())
        } else {
            GraphicalReportHandler::new()
        };

        // Absolutizes `p` in place. Skips the extra allocation `absolute()`
        // would otherwise add on top of `cwd.join()` when `p` is already
        // absolute (a common case: many CI/automation scripts pass
        // absolute paths already).
        let absolutize = |p: &Path| -> std::io::Result<PathBuf> {
            if p.is_absolute() {
                absolute(p)
            } else {
                absolute(self.cwd.join(p))
            }
        };

        let mut override_builder = None;

        if !ignore_options.no_ignore {
            let mut builder = OverrideBuilder::new(&self.cwd);

            if !ignore_options.ignore_pattern.is_empty() {
                for pattern in &ignore_options.ignore_pattern {
                    // Meaning of ignore pattern is reversed
                    // <https://docs.rs/ignore/latest/ignore/overrides/struct.OverrideBuilder.html#method.add>
                    let pattern = format!("!{pattern}");
                    builder.add(&pattern).unwrap();
                }
            }

            let builder = builder.build().unwrap();

            // The ignore crate whitelists explicit paths, but priority
            // should be given to the ignore file. Many users lint
            // automatically and pass a list of changed files explicitly.
            // To accommodate this, unless `--no-ignore` is passed,
            // pre-filter the paths.
            if !paths.is_empty() {
                let (ignore, _err) = Gitignore::new(&ignore_options.ignore_path);

                paths.retain_mut(|p| {
                    // Try to prepend cwd to all paths
                    let Ok(mut path) = absolutize(p) else {
                        return false;
                    };

                    std::mem::swap(p, &mut path);

                    if path.is_dir() {
                        true
                    } else {
                        !(builder.matched(p, false).is_ignore()
                            || ignore.matched(path, false).is_ignore())
                    }
                });
            }

            override_builder = Some(builder);
        } else if !paths.is_empty() {
            // The block above also absolutizes explicit CLI paths as a
            // side effect of pre-filtering them against ignore rules, but
            // `--no-ignore` skips that block entirely, so relative paths
            // would otherwise reach downstream consumers (e.g. the
            // type-aware tsgolint integration, which requires absolute
            // paths and panics on a relative one) unabsolutized.
            paths.retain_mut(|p| {
                let Ok(mut path) = absolutize(p) else {
                    return false;
                };
                std::mem::swap(p, &mut path);
                true
            });
        }

        if paths.is_empty() && provided_path_count == 0 {
            paths.push(self.cwd.clone());
        }

        // The walker never filters gitignored walk roots (including roots inside a gitignored directory).
        // NOTE: Applied regardless of `--no-ignore`.
        // Currently it only disables Oxlint's own ignore sources (`.eslintignore`, `--ignore-path`, `--ignore-pattern`),
        // not git's. (Aligns with during walk filtering behavior)
        let mut gitignore_checker = GitignoreChecker::new();
        paths.retain(|p| !gitignore_checker.is_gitignored_walk_root(p, &self.cwd));

        // If explicit paths were provided but all have been filtered,
        // or the default cwd target is gitignored, return early.
        if paths.is_empty() {
            if debug_files {
                return crate::mode::run_debug_files(
                    std::iter::empty::<&Path>(),
                    &self.cwd,
                    stdout,
                );
            }

            return Self::handle_no_files_found(
                stdout,
                &output_formatter,
                now,
                None,
                misc_options.no_error_on_unmatched_pattern,
            );
        }

        let walker = Walk::new(&paths, &self.cwd, &ignore_options, override_builder);
        let mut paths = walker.paths();

        // NAPI tests build `oxlint` with `testing` feature enabled.
        // In NAPI tests, sort file paths if oxlint is run with `--threads 1`.
        // This guarantees files are linted in a deterministic order.
        //
        // Note: Sorting paths would not be sufficient to guarantee deterministic linting order unless
        // `--threads 1` is also used, because otherwise linting happens in parallel on multiple threads,
        // which also produces non-determinism.
        if cfg!(feature = "testing") && misc_options.threads == Some(1) {
            paths.sort_unstable();
        }

        let mut external_plugin_store = ExternalPluginStore::new(self.external_linter.is_some());

        // Setup JS workspace before loading any configs (config parsing can load JS plugins).
        if let Some(external_linter) = &external_linter {
            let res = (external_linter.create_workspace)(self.cwd.to_string_lossy().into_owned());

            if let Err(err) = res {
                print_and_flush_stdout(stdout, &format!("Failed to setup JS workspace:\n{err}\n"));
                return CliRunResult::JsPluginWorkspaceSetupFailed;
            }
        }

        let search_for_nested_configs = !disable_nested_config &&
            // If the `--config` option is explicitly passed, we should not search for nested config files
            // as the passed config file takes absolute precedence.
            basic_options.config.is_none() &&
            !misc_options.print_config &&
            !self.options.list_rules &&
            config_discovery().nested_configs();

        let config_result = {
            let mut config_loader =
                ConfigLoader::new(external_linter, &mut external_plugin_store, &filters, None);

            config_loader.load_root_and_nested(
                &self.cwd,
                basic_options.config.as_ref(),
                &paths,
                search_for_nested_configs,
            )
        };

        let (mut root_config, nested_configs, nested_ignore_patterns) = match config_result {
            Ok(loaded) => (loaded.root, loaded.nested, loaded.nested_ignore_patterns),
            Err(error) => {
                match error {
                    CliConfigLoadError::RootConfig(error) => {
                        print_and_flush_stdout(
                            stdout,
                            &format!(
                                "Failed to parse oxlint configuration file.\n{}\n",
                                render_report(&handler, &error)
                            ),
                        );
                    }
                    CliConfigLoadError::NestedConfigs(errors) => {
                        if let Some(error) = errors.into_iter().next() {
                            let message = match &error {
                                ConfigLoadError::Parse { path, error } => {
                                    format!(
                                        "Failed to parse oxlint configuration file at {}.\n{}\n",
                                        path.to_string_lossy().cow_replace('\\', "/"),
                                        render_report(&handler, error)
                                    )
                                }
                                ConfigLoadError::Build { path, error } => {
                                    format!(
                                        "Failed to build configuration from {}.\n{}\n",
                                        path.to_string_lossy().cow_replace('\\', "/"),
                                        render_report(
                                            &handler,
                                            &OxcDiagnostic::error(error.clone())
                                        )
                                    )
                                }
                                ConfigLoadError::JsConfigFileFoundButJsRuntimeNotAvailable => {
                                    "Error: JavaScript/TypeScript config files found but JS runtime not available.\n\
                                     This is an experimental feature that requires running oxlint via Node.js.\n\
                                     Please use JSON config files (.oxlintrc.json or .oxlintrc.jsonc) instead, or run oxlint via the npm package.\n".to_string()
                                }
                                ConfigLoadError::Diagnostic(error) => {
                                    let report = render_report(&handler, error);
                                    format!("Failed to parse oxlint configuration file.\n{report}\n")
                                }
                            };
                            print_and_flush_stdout(stdout, &message);
                        }
                    }
                }

                return CliRunResult::InvalidOptionConfig;
            }
        };

        materialize_default_plugins(&mut root_config);
        let mut plugins = root_config.plugins.unwrap_or_default();
        enable_plugins.apply_overrides(&mut plugins);
        root_config.plugins = Some(plugins);

        let config_builder = match ConfigStoreBuilder::from_oxlintrc(
            false,
            root_config.clone(),
            external_linter,
            &mut external_plugin_store,
            None,
        ) {
            Ok(builder) => builder,
            Err(e) => {
                print_and_flush_stdout(
                    stdout,
                    &format!(
                        "Failed to parse oxlint configuration file.\n{}\n",
                        render_config_builder_error(&handler, e)
                    ),
                );
                return CliRunResult::InvalidOptionConfig;
            }
        }
        .with_filters(&filters);

        if misc_options.print_config {
            return crate::mode::run_print_config(&config_builder, root_config, stdout);
        }

        let lint_config = match config_builder.build(&mut external_plugin_store) {
            Ok(config) => config,
            Err(e) => {
                print_and_flush_stdout(
                    stdout,
                    &format!(
                        "Failed to build configuration.\n{}\n",
                        render_config_builder_error(&handler, e)
                    ),
                );
                return CliRunResult::InvalidOptionConfig;
            }
        };

        if self.options.list_rules {
            return crate::mode::run_rules(&lint_config, &output_formatter, stdout);
        }

        let ignore_matcher = LintIgnoreMatcher::new(
            &root_config.ignore_patterns,
            // Without a config file there are no patterns and the root is never consulted,
            // so the CWD fallback is an arbitrary placeholder.
            root_config.dir().unwrap_or(&self.cwd),
            nested_ignore_patterns,
        );

        let files_to_lint = paths
            .into_iter()
            .filter(|path| !ignore_matcher.should_ignore(Path::new(path)))
            .collect::<Vec<Arc<OsStr>>>();

        if debug_files {
            return crate::mode::run_debug_files(
                files_to_lint.iter().map(|path| Path::new(path.as_ref())),
                &self.cwd,
                stdout,
            );
        }

        // If no external rules, discard `ExternalLinter`
        let mut external_linter = self.external_linter;
        if external_plugin_store.is_empty() {
            external_linter = None;
        }

        // TODO(refactor): pull this into a shared function, so that the language server can use
        // the same functionality.
        let use_cross_module = lint_config.plugins().has_import()
            || nested_configs
                .values()
                .any(|config| config.plugins().has_import());
        let mut options =
            LintServiceOptions::new(self.cwd.clone()).with_cross_module(use_cross_module);
        if let Some(language) = lang {
            options = options.with_source_type(language.source_type());
        }

        let mut suppression_manager = SuppressionManager::load(
            options.cwd(),
            "oxlint-suppressions.json",
            suppression_options.suppress_all,
            suppression_options.prune_suppressions || fix_options.is_enabled(),
        );

        let config_store = ConfigStore::new(lint_config, nested_configs, external_plugin_store);
        let type_check_only = self.options.type_check_only;
        let type_aware =
            type_check_only || self.options.type_aware || config_store.type_aware_enabled();
        let type_check =
            type_check_only || self.options.type_check || config_store.type_check_enabled();
        if type_check && !type_aware {
            print_and_flush_stdout(
                stdout,
                "The `--type-check` option requires type-aware linting.\nUse `--type-aware --type-check` or enable `options.typeAware` in your config.\n",
            );
            return CliRunResult::InvalidOptionTypeCheckWithoutTypeAware;
        }
        if type_check_only && fix_options.is_enabled() {
            print_and_flush_stdout(
                stdout,
                "The `--type-check-only` option cannot be used with fix flags.\nRemove `--fix`, `--fix-suggestions`, and `--fix-dangerously`.\n",
            );
            return CliRunResult::InvalidOptionTypeCheckOnlyWithFix;
        }
        if type_check_only
            && (suppression_options.suppress_all || suppression_options.prune_suppressions)
        {
            print_and_flush_stdout(
                stdout,
                "The `--type-check-only` option cannot be used with suppression update flags.\nRemove `--suppress-all` and `--prune-suppressions`.\n",
            );
            return CliRunResult::InvalidOptionTypeCheckOnlyWithSuppressionUpdate;
        }
        let deny_warnings = warning_options.deny_warnings || config_store.deny_warnings();
        let max_warnings = warning_options.max_warnings.or(config_store.max_warnings());

        // Only propagate Warn/Deny; treat Allow (off) as disabling reports.
        let report_unused_directives = if type_check_only {
            None
        } else {
            match inline_config_options.report_unused_directives {
                ReportUnusedDirectives::WithoutSeverity(true) => Some(AllowWarnDeny::Warn),
                ReportUnusedDirectives::WithSeverity(Some(severity)) if severity.is_warn_deny() => {
                    Some(severity)
                }
                ReportUnusedDirectives::WithSeverity(Some(_)) => None,
                _ => match config_store.report_unused_disable_directives() {
                    Some(severity) if severity.is_warn_deny() => Some(severity),
                    _ => None,
                },
            }
        };
        let (mut diagnostic_service, tx_error) = Self::get_diagnostic_service(
            &output_formatter,
            &warning_options,
            &misc_options,
            max_warnings,
        );

        // Send JS plugins config to JS side
        if let Some(external_linter) = &external_linter {
            let res = config_store.external_plugin_store().setup_rule_configs(
                self.cwd.to_string_lossy().into_owned(),
                None,
                external_linter,
            );
            if let Err(err) = res {
                print_and_flush_stdout(
                    stdout,
                    &format!("Failed to setup JS plugin options:\n{err}\n"),
                );
                return CliRunResult::InvalidOptionConfig;
            }
        }

        let linter = Linter::new(LintOptions::default(), config_store, external_linter)
            .with_fix(fix_options.fix_kind())
            .with_report_unused_directives(report_unused_directives);

        let number_of_files = files_to_lint.len();
        let tsconfig = basic_options.tsconfig;
        if let Some(path) = tsconfig.as_ref() {
            if path.is_file() {
                options = options.with_tsconfig(path);
            } else {
                let path = if path.is_relative() {
                    options.cwd().join(path)
                } else {
                    path.clone()
                };

                print_and_flush_stdout(
                    stdout,
                    &format!(
                        "The tsconfig file {:?} does not exist, Please provide a valid tsconfig file.\n",
                        path.to_string_lossy().cow_replace('\\', "/")
                    ),
                );

                return CliRunResult::InvalidOptionTsConfig;
            }
        }

        let number_of_rules = if type_check_only {
            None
        } else {
            linter
                .number_of_rules(type_aware)
                .map(|count| count + self.arkts_config.rules.len())
        };

        if number_of_files == 0 {
            return Self::handle_no_files_found(
                stdout,
                &output_formatter,
                now,
                number_of_rules,
                misc_options.no_error_on_unmatched_pattern,
            );
        }

        let cwd = options.cwd().to_path_buf();

        // Create the LintRunner
        // TODO: Add a warning message if `tsgolint` cannot be found, but type-aware rules are enabled
        let lint_runner = match LintRunner::builder(options, linter)
            .with_type_aware(type_aware)
            .with_type_check(type_check)
            .with_silent(misc_options.silent)
            .with_fix_kind(fix_options.fix_kind())
            .with_type_check_only(type_check_only)
            .with_timings(debug_timings)
            .build()
        {
            Ok(runner) => runner,
            Err(err) => {
                print_and_flush_stdout(stdout, &format!("{err}\n"));
                return CliRunResult::TsGoLintError;
            }
        };

        let diff_manager = suppression_manager.build_diff();

        let rule_timing_store = debug_timings.then(RuleTimingStore::new);
        let lint_result = if let Some(rule_timing_store) = &rule_timing_store {
            lint_runner.lint_files::<true>(
                &files_to_lint,
                tx_error.clone(),
                &diff_manager,
                Some(rule_timing_store),
            )
        } else {
            lint_runner.lint_files::<false>(&files_to_lint, tx_error.clone(), &diff_manager, None)
        };

        match lint_result {
            Ok(lint_runner) => {
                lint_runner.report_unused_directives(report_unused_directives, &tx_error);
            }
            Err(err) => {
                print_and_flush_stdout(stdout, &format!("{err}\n"));
                return CliRunResult::TsGoLintError;
            }
        }

        // A suppression file can contain regular lint rules that were not run in type-check-only
        // mode, so its runtime diff is incomplete and cannot be used to validate the baseline.
        let result = if type_check_only {
            Ok(())
        } else {
            suppression_manager.finalize(diff_manager, &tx_error, &cwd)
        };
        let suppress_all_succeeded = suppression_options.suppress_all && result.is_ok();

        if let Err(err) = Self::lint_arkts_files(
            &cwd,
            &self.arkts_config,
            &files_to_lint,
            &tx_error,
            lang.map(|language| language.source_type()),
        ) {
            print_and_flush_stdout(stdout, &format!("{err}\n"));
            return CliRunResult::InvalidOptionConfig;
        }
        drop(tx_error);

        let diagnostic_result = diagnostic_service.run(stdout);

        let oxlint_suppression_file_action = if let Err(report_suppression_error) = result {
            OxlintSuppressionFileAction::UnableToPerformFsOperation(report_suppression_error)
        } else {
            suppression_manager.file_action
        };

        let has_unpruned_suppressions = matches!(
            oxlint_suppression_file_action,
            OxlintSuppressionFileAction::HasUnprunedSuppressions
        );

        if let Some(end) = output_formatter.lint_command_info(&LintCommandInfo {
            number_of_files,
            number_of_rules,
            threads_count: rayon::current_num_threads(),
            start_time: now.elapsed(),
            oxlint_suppression_file_action,
            rule_timings: rule_timing_store.as_ref().map(|store| {
                crate::output_formatter::RuleTimings {
                    records: store.collect(),
                    js_plugin_runtime: store.js_plugin_runtime(),
                }
            }),
        }) {
            print_and_flush_stdout(stdout, &end);
        }

        // When --suppress-all is used and the file was written successfully,
        // exit with success (matching ESLint behavior: suppressing is a success action).
        if suppress_all_succeeded {
            return CliRunResult::LintSucceeded;
        }

        if has_unpruned_suppressions {
            return CliRunResult::LintUnprunedSuppressions;
        }

        if diagnostic_result.errors_count() > 0 {
            CliRunResult::LintFoundErrors
        } else if deny_warnings && diagnostic_result.warnings_count() > 0 {
            CliRunResult::LintNoWarningsAllowed
        } else if diagnostic_result.max_warnings_exceeded() {
            CliRunResult::LintMaxWarningsExceeded
        } else {
            CliRunResult::LintSucceeded
        }
    }
}

impl OxkLintRunner {
    fn get_diagnostic_service(
        reporter: &OutputFormatter,
        warning_options: &WarningOptions,
        misc_options: &MiscOptions,
        max_warnings: Option<usize>,
    ) -> (DiagnosticService, DiagnosticSender) {
        let (service, sender) = DiagnosticService::new(reporter.get_diagnostic_reporter());
        (
            service
                .with_quiet(warning_options.quiet)
                .with_silent(misc_options.silent)
                .with_max_warnings(max_warnings),
            sender,
        )
    }

    fn lint_arkts_files(
        cwd: &Path,
        arkts_config: &ArktsLintConfig,
        files_to_lint: &[Arc<OsStr>],
        tx_error: &DiagnosticSender,
        source_type: Option<SourceType>,
    ) -> Result<(), String> {
        if arkts_config.is_empty() {
            return Ok(());
        }

        for file in files_to_lint {
            let path = Path::new(file.as_ref());
            if path.extension().and_then(|ext| ext.to_str()) != Some("ets") {
                continue;
            }

            let source_text = fs::read_to_string(path).map_err(|err| {
                format!(
                    "Failed to read ArkTS source file `{}`: {err}",
                    path.display()
                )
            })?;
            let diagnostics = arkts::lint_standalone_source(
                path,
                &source_text,
                &arkts_config.rules,
                cwd,
                source_type,
            )?;
            if diagnostics.is_empty() {
                continue;
            }

            let diagnostics = diagnostics
                .into_iter()
                .map(|diagnostic| {
                    let base = match diagnostic.severity {
                        arkts::StandaloneSeverity::Warn => OxcDiagnostic::warn(diagnostic.message),
                        arkts::StandaloneSeverity::Error => {
                            OxcDiagnostic::error(diagnostic.message)
                        }
                    };
                    base.with_error_code("arkts", diagnostic.rule_name)
                        .with_label(Span::new(diagnostic.start, diagnostic.end))
                })
                .collect();
            let diagnostics =
                DiagnosticService::wrap_diagnostics(cwd, path, &source_text, diagnostics);
            tx_error
                .send(diagnostics)
                .map_err(|err| format!("Failed to send ArkTS diagnostics: {err}"))?;
        }

        Ok(())
    }

    fn handle_no_files_found(
        stdout: &mut dyn Write,
        output_formatter: &OutputFormatter,
        now: Instant,
        number_of_rules: Option<usize>,
        no_error_on_unmatched_pattern: bool,
    ) -> CliRunResult {
        if !no_error_on_unmatched_pattern {
            print_and_flush_stdout(
                stdout,
                "No files found to lint. Please check your paths and ignore patterns.\n",
            );
        }

        if let Some(end) = output_formatter.lint_command_info(&LintCommandInfo {
            number_of_files: 0,
            number_of_rules,
            threads_count: rayon::current_num_threads(),
            start_time: now.elapsed(),
            oxlint_suppression_file_action: OxlintSuppressionFileAction::None,
            rule_timings: None,
        }) {
            print_and_flush_stdout(stdout, &end);
        }

        if no_error_on_unmatched_pattern {
            CliRunResult::LintSucceeded
        } else {
            CliRunResult::LintNoFilesFound
        }
    }

    // moved into a separate function for readability, but it's only ever used
    // in one place.
    fn get_filters(
        filters_arg: Vec<(AllowWarnDeny, String)>,
    ) -> Result<Vec<LintFilter>, (CliRunResult, String)> {
        let mut filters = Vec::with_capacity(filters_arg.len());

        for (severity, filter_arg) in filters_arg {
            match LintFilter::new(severity, filter_arg) {
                Ok(filter) => {
                    filters.push(filter);
                }
                Err(InvalidFilterKind::Empty) => {
                    return Err((
                        CliRunResult::InvalidOptionSeverityWithoutFilter,
                        format!("Cannot {severity} an empty filter.\n"),
                    ));
                }
                Err(InvalidFilterKind::PluginMissing(filter)) => {
                    return Err((
                        CliRunResult::InvalidOptionSeverityWithoutPluginName,
                        format!(
                            "Failed to {severity} filter {filter}: Plugin name is missing. Expected <plugin>/<rule>\n"
                        ),
                    ));
                }
                Err(InvalidFilterKind::RuleMissing(filter)) => {
                    return Err((
                        CliRunResult::InvalidOptionSeverityWithoutRuleName,
                        format!(
                            "Failed to {severity} filter {filter}: Rule name is missing. Expected <plugin>/<rule>\n"
                        ),
                    ));
                }
            }
        }

        Ok(filters)
    }
}

pub fn print_and_flush_stdout(stdout: &mut dyn Write, message: &str) {
    stdout
        .write_all(message.as_bytes())
        .or_else(check_for_writer_error)
        .unwrap();
    stdout.flush().or_else(check_for_writer_error).unwrap();
}

fn check_for_writer_error(error: std::io::Error) -> Result<(), std::io::Error> {
    // Do not panic when the process is killed (e.g. piping into `less`).
    if matches!(
        error.kind(),
        ErrorKind::Interrupted | ErrorKind::BrokenPipe | ErrorKind::WouldBlock
    ) {
        Ok(())
    } else {
        Err(error)
    }
}

fn render_report(handler: &GraphicalReportHandler, diagnostic: &OxcDiagnostic) -> String {
    let mut err = String::new();
    handler.render_report(&mut err, diagnostic).unwrap();
    err
}

fn render_config_builder_error(
    handler: &GraphicalReportHandler,
    error: ConfigBuilderError,
) -> String {
    match error {
        ConfigBuilderError::RuleConfigurationErrors { errors } => errors
            .iter()
            .map(|e| render_report(handler, &OxcDiagnostic::error(e.to_string())))
            .collect::<String>(),
        _ => render_report(handler, &OxcDiagnostic::error(error.to_string())),
    }
}
