use std::{
    ffi::OsString,
    fs,
    io::BufWriter,
    path::{Path, PathBuf},
    sync::Once,
    time::{SystemTime, UNIX_EPOCH},
};

use oxlint::cli::{CliRunResult, CliRunner, LintCommand, init_miette, init_tracing, lint_command};
use serde_json::{Map, Value, map::Entry};

mod arkts;
#[cfg(feature = "napi")]
mod napi_lint;

#[cfg(feature = "napi")]
pub use napi_lint::{
    JsCreateWorkspaceCb, JsDestroyWorkspaceCb, JsLintFileCb, JsLoadJsConfigsCb, JsLoadPluginCb,
    JsSetupRuleConfigsCb, lint_args_with_plugins,
};

pub fn lint_args(args: Vec<OsString>) -> bool {
    let prepared = match prepare_arkts_config(args) {
        Ok(prepared) => prepared,
        Err(err) => {
            eprintln!("{err}");
            return false;
        }
    };

    let command = {
        let parser = lint_command();
        match parser.run_inner(&*prepared.args) {
            Ok(command) => command,
            Err(err) => {
                err.print_message(100);
                return err.exit_code() == 0;
            }
        }
    };

    init_tracing();

    if command.lsp {
        eprintln!("oxk lint --lsp is not supported by the embedded lint runner.");
        return false;
    }

    init_miette();

    handle_threads_once(&command);

    let mut stdout = BufWriter::new(std::io::stdout());
    is_success(CliRunner::new(command, Some(arkts::create_external_linter(None))).run(&mut stdout))
}

struct PreparedLintArgs {
    args: Vec<OsString>,
    _temp_config: Option<TempArktsConfig>,
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

    fn plugin_path(&self) -> PathBuf {
        self.dir.join("arkts-plugin.js")
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
            _temp_config: None,
        });
    };

    if !is_json_lint_config(&config_path) {
        return Ok(PreparedLintArgs {
            args,
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

    if !config_contains_arkts_plugin(&config) {
        return Ok(PreparedLintArgs {
            args,
            _temp_config: None,
        });
    }

    let temp_config = TempArktsConfig::new()?;
    let plugin_path = temp_config.plugin_path();
    arkts::write_placeholder_plugin(&plugin_path).map_err(|err| {
        format!(
            "Failed to write ArkTS plugin placeholder `{}`: {err}",
            plugin_path.display()
        )
    })?;
    rewrite_arkts_plugin_config(&mut config, &plugin_path);

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

fn resolve_from_cwd(path: PathBuf) -> PathBuf {
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

fn config_contains_arkts_plugin(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };

    object
        .get("plugins")
        .and_then(Value::as_array)
        .is_some_and(|plugins| {
            plugins
                .iter()
                .any(|plugin| plugin.as_str() == Some(arkts::ARKTS_PLUGIN_NAME))
        })
        || object
            .get("overrides")
            .and_then(Value::as_array)
            .is_some_and(|overrides| overrides.iter().any(config_contains_arkts_plugin))
}

fn rewrite_arkts_plugin_config(value: &mut Value, plugin_path: &Path) -> bool {
    let Some(object) = value.as_object_mut() else {
        return false;
    };

    let mut changed = false;
    if let Some(Value::Array(plugins)) = object.get_mut("plugins") {
        let before_len = plugins.len();
        plugins.retain(|plugin| plugin.as_str() != Some(arkts::ARKTS_PLUGIN_NAME));
        if plugins.len() != before_len {
            ensure_arkts_js_plugin(object, plugin_path);
            changed = true;
        }
    }

    if let Some(Value::Array(overrides)) = object.get_mut("overrides") {
        for override_config in overrides {
            changed |= rewrite_arkts_plugin_config(override_config, plugin_path);
        }
    }

    changed
}

fn ensure_arkts_js_plugin(object: &mut Map<String, Value>, plugin_path: &Path) {
    let entry = arkts::arkts_plugin_config_entry(plugin_path);
    match object.entry("jsPlugins") {
        Entry::Occupied(mut occupied) => {
            if let Value::Array(js_plugins) = occupied.get_mut() {
                if !js_plugins.iter().any(is_arkts_js_plugin_entry) {
                    js_plugins.push(entry);
                }
            } else {
                occupied.insert(Value::Array(vec![entry]));
            }
        }
        Entry::Vacant(vacant) => {
            vacant.insert(Value::Array(vec![entry]));
        }
    }
}

fn is_arkts_js_plugin_entry(value: &Value) -> bool {
    value.as_str() == Some(arkts::ARKTS_PLUGIN_NAME)
        || value.as_object().is_some_and(|object| {
            object
                .get("name")
                .and_then(Value::as_str)
                .or_else(|| object.get("specifier").and_then(Value::as_str))
                == Some(arkts::ARKTS_PLUGIN_NAME)
        })
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
