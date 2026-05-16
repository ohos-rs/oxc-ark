use std::{
    ffi::OsString,
    fs,
    io::BufWriter,
    path::{Path, PathBuf},
    sync::Once,
    time::{SystemTime, UNIX_EPOCH},
};

use oxc_diagnostics::OxcDiagnostic;
use oxc_linter::{ExternalLinter, Oxlintrc};
use oxlint::cli::{CliRunResult, LintCommand, init_miette, init_tracing, lint_command};
use serde_json::Value;

mod arkts;
mod config_loader;
mod lsp;
mod mode;
#[cfg(feature = "napi")]
mod napi_lint;
mod output_formatter;
mod runner;
mod schema;
mod walk;

const DEFAULT_OXLINTRC_NAME: &str = ".oxlintrc.json";
const DEFAULT_JSONC_OXLINTRC_NAME: &str = ".oxlintrc.jsonc";
const DEFAULT_TS_OXLINTRC_NAME: &str = "oxlint.config.ts";

#[cfg(feature = "napi")]
pub use napi_lint::{
    JsCreateWorkspaceCb, JsDestroyWorkspaceCb, JsLintFileCb, JsLoadJsConfigsCb, JsLoadPluginCb,
    JsSetupRuleConfigsCb, lint_args_with_plugins,
};

pub fn lint_args(args: Vec<OsString>) -> bool {
    if let Some(success) = handle_schema_args(&args) {
        return success;
    }

    let command = match parse_lint_command(&args) {
        Ok(command) => command,
        Err(success) => return success,
    };

    init_tracing();

    if command.lsp {
        let config_path = command.basic_options.config.clone().map(resolve_from_cwd);
        return run_lsp_server(None, config_path);
    }

    let prepared = match prepare_arkts_config(args) {
        Ok(prepared) => prepared,
        Err(err) => {
            eprintln!("{err}");
            return false;
        }
    };

    let command = match parse_lint_command(&prepared.args) {
        Ok(command) => command,
        Err(success) => return success,
    };

    init_miette();

    handle_threads_once(&command);

    run_lint_command(command, prepared.arkts, None)
}

pub(crate) fn parse_lint_command(args: &[OsString]) -> Result<LintCommand, bool> {
    let parser = lint_command();
    match parser.run_inner(args) {
        Ok(command) => Ok(command),
        Err(err) => {
            err.print_message(100);
            Err(err.exit_code() == 0)
        }
    }
}

fn handle_schema_args(args: &[OsString]) -> Option<bool> {
    let args = args
        .iter()
        .map(|arg| arg.to_string_lossy())
        .collect::<Vec<_>>();
    match args.as_slice() {
        [arg] if *arg == "--print-config-schema" || *arg == "--schema-json" => {
            println!("{}", schema::configuration_schema_json());
            Some(true)
        }
        [arg, path] if *arg == "--write-config-schema" => {
            match schema::write_configuration_schema(Path::new(path.as_ref())) {
                Ok(()) => Some(true),
                Err(err) => {
                    eprintln!("Failed to write configuration schema `{path}`: {err}");
                    Some(false)
                }
            }
        }
        [arg] if *arg == "--write-config-schema" => {
            eprintln!("--write-config-schema requires an output path.");
            Some(false)
        }
        _ => None,
    }
}

struct PreparedLintArgs {
    args: Vec<OsString>,
    arkts: ArktsLintConfig,
    _temp_config: Option<TempArktsConfig>,
}

#[derive(Clone, Default)]
pub(crate) struct ArktsLintConfig {
    pub(crate) rules: Vec<arkts::StandaloneRuleConfig>,
}

impl ArktsLintConfig {
    pub(crate) fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }
}

struct TempArktsConfig {
    dir: PathBuf,
}

impl TempArktsConfig {
    fn new() -> Result<Self, String> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|err| format!("Failed to create ArkTS lint config timestamp: {err}"))?
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("oxk-arkts-lint-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&dir)
            .map_err(|err| format!("Failed to create temporary ArkTS lint config dir: {err}"))?;
        Ok(Self { dir })
    }

    fn config_path(&self) -> PathBuf {
        self.dir.join("oxlintrc.json")
    }
}

impl Drop for TempArktsConfig {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn prepare_arkts_config(args: Vec<OsString>) -> Result<PreparedLintArgs, String> {
    let Some(config_path) = find_lint_config_path(&args) else {
        return Ok(PreparedLintArgs {
            args,
            arkts: ArktsLintConfig::default(),
            _temp_config: None,
        });
    };

    if !is_json_lint_config(&config_path) {
        return Ok(PreparedLintArgs {
            args,
            arkts: ArktsLintConfig::default(),
            _temp_config: None,
        });
    }

    let mut json = fs::read_to_string(&config_path).map_err(|err| {
        format!(
            "Failed to read oxlint configuration `{}`: {err}",
            config_path.display()
        )
    })?;
    if config_path.extension().and_then(|ext| ext.to_str()) == Some("jsonc") {
        json_strip_comments::strip(&mut json).map_err(|err| {
            format!(
                "Failed to strip comments from oxlint configuration `{}`: {err}",
                config_path.display()
            )
        })?;
    }
    let mut config: Value = serde_json::from_str(&json).map_err(|err| {
        format!(
            "Failed to parse oxlint configuration `{}`: {err}",
            config_path.display()
        )
    })?;

    let mut arkts_config = ArktsLintConfig::default();
    if !rewrite_arkts_builtin_config(&mut config, &mut arkts_config)? {
        return Ok(PreparedLintArgs {
            args,
            arkts: arkts_config,
            _temp_config: None,
        });
    }

    let temp_config = TempArktsConfig::new()?;
    let temp_config_path = temp_config.config_path();
    let config_text = serde_json::to_string_pretty(&config)
        .map_err(|err| format!("Failed to serialize rewritten ArkTS lint config: {err}"))?;
    fs::write(&temp_config_path, config_text).map_err(|err| {
        format!(
            "Failed to write rewritten ArkTS lint config `{}`: {err}",
            temp_config_path.display()
        )
    })?;

    Ok(PreparedLintArgs {
        args: replace_config_arg(args, &temp_config_path),
        arkts: arkts_config,
        _temp_config: Some(temp_config),
    })
}

fn find_lint_config_path(args: &[OsString]) -> Option<PathBuf> {
    for (index, arg) in args.iter().enumerate() {
        let arg = arg.to_string_lossy();
        if arg == "--config" || arg == "-c" {
            return args.get(index + 1).map(PathBuf::from).map(resolve_from_cwd);
        }
        if let Some(value) = arg.strip_prefix("--config=") {
            return Some(resolve_from_cwd(PathBuf::from(value)));
        }
    }

    [".oxlintrc.json", ".oxlintrc.jsonc"]
        .into_iter()
        .map(PathBuf::from)
        .map(resolve_from_cwd)
        .find(|path| path.is_file())
}

pub(crate) fn resolve_from_cwd(path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

fn is_json_lint_config(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("json" | "jsonc")
    )
}

pub(crate) fn load_oxlintrc_and_arkts_from_file(
    path: &Path,
) -> Result<(Oxlintrc, ArktsLintConfig), OxcDiagnostic> {
    let mut json_text = fs::read_to_string(path).map_err(|err| {
        OxcDiagnostic::error(format!(
            "Failed to parse config {} with error {err:?}",
            path.display()
        ))
    })?;

    json_strip_comments::strip(&mut json_text).map_err(|err| {
        OxcDiagnostic::error(format!(
            "Failed to parse jsonc file {}: {err:?}",
            path.display()
        ))
    })?;

    let mut value = serde_json::from_str::<Value>(&json_text).map_err(|err| {
        let ext = path.extension().and_then(std::ffi::OsStr::to_str);
        let err = match ext {
            Some("json" | "jsonc") => err.to_string(),
            Some(_) => "Only JSON configuration files are supported".to_string(),
            None => {
                format!("{err}, if the configuration is not a JSON file, please use JSON instead.")
            }
        };
        OxcDiagnostic::error(format!(
            "Failed to parse oxlint config {}.\n{err}",
            path.display()
        ))
    })?;

    let mut arkts_config = ArktsLintConfig::default();
    rewrite_arkts_builtin_config(&mut value, &mut arkts_config).map_err(OxcDiagnostic::error)?;

    let mut config = serde_json::from_value::<Oxlintrc>(value).map_err(|err| {
        OxcDiagnostic::error(format!("Failed to parse config with error {err:?}"))
    })?;

    config.path = path.to_path_buf();
    let config_dir = config
        .path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    config.set_config_dir(&config_dir);

    Ok((config, arkts_config))
}

pub(crate) fn rewrite_arkts_builtin_config(
    value: &mut Value,
    arkts_config: &mut ArktsLintConfig,
) -> Result<bool, String> {
    let Some(object) = value.as_object_mut() else {
        return Ok(false);
    };

    let mut changed = false;
    if let Some(Value::Array(plugins)) = object.get_mut("plugins") {
        let before_len = plugins.len();
        plugins.retain(|plugin| plugin.as_str() != Some(arkts::ARKTS_PLUGIN_NAME));
        if plugins.len() != before_len {
            changed = true;
        }
    }

    if let Some(Value::Object(rules)) = object.get_mut("rules") {
        let arkts_rule_keys = rules
            .keys()
            .filter(|rule_name| rule_name.starts_with("arkts/"))
            .cloned()
            .collect::<Vec<_>>();
        for rule_key in arkts_rule_keys {
            let raw_rule_config = rules
                .remove(&rule_key)
                .expect("rule key collected from this map should exist");
            if let Some(rule_config) = parse_arkts_rule_config(&rule_key, &raw_rule_config)? {
                arkts_config.rules.push(rule_config);
            }
            changed = true;
        }
    }

    if let Some(Value::Array(overrides)) = object.get_mut("overrides") {
        for override_config in overrides {
            changed |= rewrite_arkts_builtin_config(override_config, arkts_config)?;
        }
    }

    Ok(changed)
}

fn parse_arkts_rule_config(
    full_rule_name: &str,
    raw_rule_config: &Value,
) -> Result<Option<arkts::StandaloneRuleConfig>, String> {
    let rule_name = full_rule_name
        .strip_prefix("arkts/")
        .ok_or_else(|| format!("Invalid ArkTS lint rule name `{full_rule_name}`."))?;
    if !arkts::is_rule_name(rule_name) {
        return Err(format!("Unknown ArkTS lint rule `{full_rule_name}`."));
    }

    let (severity_value, options) = match raw_rule_config {
        Value::Array(values) => {
            let Some(severity) = values.first() else {
                return Err(format!(
                    "ArkTS lint rule `{full_rule_name}` must include a severity."
                ));
            };
            (severity, values.iter().skip(1).cloned().collect::<Vec<_>>())
        }
        value => (value, Vec::new()),
    };

    let Some(severity) = parse_arkts_severity(full_rule_name, severity_value)? else {
        return Ok(None);
    };

    Ok(Some(arkts::StandaloneRuleConfig {
        name: rule_name.to_string(),
        severity,
        options,
    }))
}

fn parse_arkts_severity(
    rule_name: &str,
    value: &Value,
) -> Result<Option<arkts::StandaloneSeverity>, String> {
    match value {
        Value::String(value) => match value.as_str() {
            "off" | "0" => Ok(None),
            "warn" | "warning" | "1" => Ok(Some(arkts::StandaloneSeverity::Warn)),
            "error" | "deny" | "2" => Ok(Some(arkts::StandaloneSeverity::Error)),
            _ => Err(format!(
                "ArkTS lint rule `{rule_name}` has invalid severity `{value}`."
            )),
        },
        Value::Number(value) if value.as_u64() == Some(0) => Ok(None),
        Value::Number(value) if value.as_u64() == Some(1) => {
            Ok(Some(arkts::StandaloneSeverity::Warn))
        }
        Value::Number(value) if value.as_u64() == Some(2) => {
            Ok(Some(arkts::StandaloneSeverity::Error))
        }
        Value::Bool(false) => Ok(None),
        Value::Bool(true) => Ok(Some(arkts::StandaloneSeverity::Error)),
        _ => Err(format!(
            "ArkTS lint rule `{rule_name}` must use severity off, warn, or error."
        )),
    }
}

fn replace_config_arg(args: Vec<OsString>, config_path: &Path) -> Vec<OsString> {
    let mut next_args = Vec::with_capacity(args.len() + 2);
    let mut replaced = false;
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        let arg_string = arg.to_string_lossy();
        if arg_string == "--config" || arg_string == "-c" {
            next_args.push(arg.clone());
            next_args.push(config_path.as_os_str().to_os_string());
            index += 2;
            replaced = true;
        } else if arg_string.starts_with("--config=") {
            next_args.push(OsString::from(format!(
                "--config={}",
                config_path.to_string_lossy()
            )));
            index += 1;
            replaced = true;
        } else {
            next_args.push(arg.clone());
            index += 1;
        }
    }

    if !replaced {
        next_args.push(OsString::from("--config"));
        next_args.push(config_path.as_os_str().to_os_string());
    }

    next_args
}

fn run_lint_command(
    command: LintCommand,
    arkts_config: ArktsLintConfig,
    external_linter: Option<ExternalLinter>,
) -> bool {
    let mut stdout = BufWriter::new(std::io::stdout());
    is_success(runner::OxkLintRunner::new(command, arkts_config, external_linter).run(&mut stdout))
}

pub(crate) fn run_lsp_server(
    external_linter: Option<ExternalLinter>,
    config_path: Option<PathBuf>,
) -> bool {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("Failed to create lint LSP runtime: {err}");
            return false;
        }
    };

    runtime.block_on(lsp::run_lsp(external_linter, config_path));
    true
}

fn handle_threads_once(command: &LintCommand) {
    static RAYON_INIT: Once = Once::new();
    RAYON_INIT.call_once(|| command.handle_threads());
}

fn is_success(result: CliRunResult) -> bool {
    matches!(
        result,
        CliRunResult::None
            | CliRunResult::PrintConfigResult
            | CliRunResult::ConfigFileInitSucceeded
            | CliRunResult::LintSucceeded
    )
}
