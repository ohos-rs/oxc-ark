// Derived from ohos-rs/oxc 230deff13c3d0baddb9560ba895ee2b6a3fc4b7e (MIT).
// Copyright (c) Oxc project contributors.

use rustc_hash::{FxBuildHasher, FxHashMap};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, de::Error};
use serde_json::Value;

use oxc_linter::{FixKind, normalize_rule_name};
use tracing::error;

use crate::config_loader::config_discovery;

#[derive(Debug, Serialize, Deserialize, Default, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum UnusedDisableDirectives {
    #[default]
    Allow,
    Warn,
    Deny,
}

#[derive(Debug, Serialize, Deserialize, Default, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum Run {
    OnSave,
    #[default]
    OnType,
}

/// Explicit language modes that cannot be inferred from a file extension.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, JsonSchema)]
pub enum Language {
    #[serde(rename = "ets-static")]
    EtsStatic,
}

impl Language {
    pub const fn source_type(self) -> oxc_span::SourceType {
        match self {
            Self::EtsStatic => oxc_span::SourceType::ets_static(),
        }
    }
}

/// LSP Options
///
/// These options can be defined for each workspace folder separately.
/// File references in the options (e.g. `configPath`, `tsConfigPath`) are resolved relative to the workspace folder.
///
/// They can be sent by the client in `initialize` or `workspace/didChangeConfiguration` requests.
/// If the client supports `workspace/configuration`, the server will request the options from the client.
///
/// Example of `initialize` request:
/// ```json
/// {
///   "processId": 123,
///   "rootUri": null,
///   "workspaceFolders": [],
///   "capabilities": {},
///   "initializationOptions": [
///     {
///       "workspaceUri": "file:///home/user/project",
///       "options": {
///         "unusedDisableDirectives": "deny",
///         "typeAware": true
///       }
///     }
///   ]
/// }
/// ```
///
/// Example of `workspace/didChangeConfiguration` request:
/// ```json
/// {
///   "settings": [
///     {
///       "workspaceUri": "file:///home/user/project",
///       "options": {
///         "unusedDisableDirectives": "deny",
///         "disableNestedConfig": true
///       }
///     }
///   ]
/// }
/// ```
#[derive(Debug, Default, Serialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LintOptions {
    /// If your editor does not support `textDocument/diagnostic`,
    /// this option handles when diagnostics are sent to the client.
    ///
    /// This option only applies to clients which use the push model. Clients which support
    /// `textDocument/diagnostic` ask for diagnostics themselves (the pull model), so `run`
    /// currently has no effect for them.
    /// See [#26613](https://github.com/oxc-project/oxc/issues/26613)
    #[schemars(with = "Option<Run>")]
    pub run: Run,
    /// Path to the config file. Similar to `--config` CLI option.
    /// If set, it disables searching for config files.
    /// An empty string is treated as unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_path: Option<String>,
    /// Path to the tsconfig file. Similar to `--tsconfig` CLI option.
    /// If set, it disables auto discovery for tsconfig files.
    /// An empty string is treated as unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ts_config_path: Option<String>,
    /// How to handle unused disable directives. By default, they are allowed and ignored.
    pub unused_disable_directives: Option<UnusedDisableDirectives>,
    /// Whether to enable/disable type-aware linting.
    /// It will override the root config's `typeAware` option if set.
    pub type_aware: Option<bool>,
    /// Explicit language grammar for `.ets` documents. Without this option,
    /// `.ets` remains ArkUI/ArkTS 1.1.
    pub language: Option<Language>,
    /// Whether to disable nested config support. Similar to `--disable-nested-config` CLI option.
    /// It gets automatically enabled when `configPath` is set.
    /// Nested config support is always disabled in Vite+ mode.
    #[schemars(with = "Option<bool>")]
    pub disable_nested_config: bool,
    /// What kind of fixes to generate for code actions.
    #[schemars(with = "Option<LintFixKindFlag>")]
    pub fix_kind: LintFixKindFlag,
    /// Customization for individual rules, allows to override the linter's diagnostics and autofix.
    /// Example of lowering the severity of "no-unused-vars" rule to "hint" and disabling autofix for it:
    /// ```json
    /// {
    ///   "rulesCustomization": {
    ///     "no-unused-vars": {
    ///       "severity": "hint",
    ///       "autofix": false
    ///     }
    ///   }
    /// }
    /// ```
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rules_customization: Option<RulesCustomization>,
}

#[derive(Debug, Default, Serialize, PartialEq, Eq, JsonSchema)]
#[serde(transparent)]
pub struct RulesCustomization {
    #[serde(flatten)]
    pub rules: FxHashMap<String, RuleCustomization>,
}

impl<'de> Deserialize<'de> for RulesCustomization {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        let Some(object) = value.as_object() else {
            return Err(D::Error::custom("rulesCustomization must be an object"));
        };

        let mut rules = FxHashMap::with_capacity_and_hasher(object.len(), FxBuildHasher);
        for (rule_name, rule_config) in object {
            let Ok(customization) = RuleCustomization::deserialize(rule_config) else {
                error!("failed to deserialize customization for rule {rule_name}, skipping.");
                continue;
            };
            rules.insert(normalize_rule_name(rule_name), customization);
        }

        Ok(Self { rules })
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum RuleCustomizationSeverity {
    Error,
    Warn,
    Info,
    Hint,
    Off,
}

#[derive(Debug, Default, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RuleCustomization {
    // note: this will not enable "off" rules; it only customizes active rules
    // it only modifies the severity of returned diagnostics, not the linter configuration
    pub severity: Option<RuleCustomizationSeverity>,
    // this flag indicates whether to include this rule in "fix all" and "fix all dangerous" code action
    pub autofix: Option<bool>,
}

#[derive(Debug, Default, Serialize, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LintFixKindFlag {
    SafeFix,
    #[default]
    SafeFixOrSuggestion,
    DangerousFix,
    DangerousFixOrSuggestion,
    None,
    All,
}

impl From<LintFixKindFlag> for FixKind {
    fn from(flag: LintFixKindFlag) -> Self {
        match flag {
            LintFixKindFlag::SafeFix => FixKind::SafeFix,
            LintFixKindFlag::SafeFixOrSuggestion => FixKind::SafeFixOrSuggestion,
            LintFixKindFlag::DangerousFix => FixKind::DangerousFix,
            LintFixKindFlag::DangerousFixOrSuggestion => FixKind::DangerousFixOrSuggestion,
            LintFixKindFlag::None => FixKind::None,
            LintFixKindFlag::All => FixKind::All,
        }
    }
}

impl LintOptions {
    /// Nested config search: off with `disableNestedConfig`, an explicit `configPath`, or in Vite+ mode.
    pub fn use_nested_configs(&self) -> bool {
        !self.disable_nested_config
            && self.config_path.is_none()
            && config_discovery().nested_configs()
    }
}

impl<'de> Deserialize<'de> for LintOptions {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        LintOptions::try_from(value).map_err(Error::custom)
    }
}

impl TryFrom<Value> for LintOptions {
    type Error = String;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        // null is treated as default options
        if value == Value::Null {
            return Ok(Self::default());
        }

        let Some(object) = value.as_object() else {
            return Err("no object passed".to_string());
        };

        // deprecated flags field
        let mut flags = FxHashMap::with_capacity_and_hasher(2, FxBuildHasher);
        if let Some(json_flags) = object.get("flags").and_then(Value::as_object) {
            if let Some(disable_nested_config) = json_flags
                .get("disable_nested_config")
                .and_then(Value::as_str)
            {
                flags.insert("disable_nested_config".to_string(), disable_nested_config);
            }

            if let Some(fix_kind) = json_flags.get("fix_kind").and_then(Value::as_str) {
                flags.insert("fix_kind".to_string(), fix_kind);
            }
        }
        Ok(Self {
            run: object
                .get("run")
                .and_then(|run| Run::deserialize(run).ok())
                .unwrap_or_default(),
            unused_disable_directives: object
                .get("unusedDisableDirectives")
                .and_then(|key| UnusedDisableDirectives::deserialize(key).ok()),
            config_path: object
                .get("configPath")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
            ts_config_path: object
                .get("tsConfigPath")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
            type_aware: object.get("typeAware").and_then(Value::as_bool),
            language: object
                .get("language")
                .and_then(|value| Language::deserialize(value).ok()),
            disable_nested_config: object
                .get("disableNestedConfig")
                .and_then(Value::as_bool)
                .unwrap_or(flags.contains_key("disable_nested_config")),
            fix_kind: object
                .get("fixKind")
                .and_then(|key| LintFixKindFlag::deserialize(key).ok())
                .unwrap_or_else(|| match flags.get("fix_kind") {
                    Some(&"safe_fix") => LintFixKindFlag::SafeFix,
                    Some(&"safe_fix_or_suggestion") => LintFixKindFlag::SafeFixOrSuggestion,
                    Some(&"dangerous_fix") => LintFixKindFlag::DangerousFix,
                    Some(&"dangerous_fix_or_suggestion") => {
                        LintFixKindFlag::DangerousFixOrSuggestion
                    }
                    Some(&"none") => LintFixKindFlag::None,
                    Some(&"all") => LintFixKindFlag::All,
                    _ => LintFixKindFlag::default(),
                }),
            rules_customization: object
                .get("rulesCustomization")
                .and_then(|key| RulesCustomization::deserialize(key).ok()),
        })
    }
}
