#[cfg(feature = "napi")]
use std::sync::{Arc, Mutex};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast_visit::{Visit, utf8_to_utf16::Utf8ToUtf16Converter, walk};
use oxc_linter::LintFileResult;
#[cfg(feature = "napi")]
use oxc_linter::{
    ExternalLinter, ExternalLinterCreateWorkspaceCb, ExternalLinterDestroyWorkspaceCb,
    ExternalLinterLintFileCb, ExternalLinterLoadPluginCb, ExternalLinterSetupRuleConfigsCb,
};
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType, Span};
use oxc_syntax::operator::{BinaryOperator, UnaryOperator};

mod system_api_versions;

use system_api_versions::{SYSTEM_API_VERSIONS, SystemApiVersion};

pub const ARKTS_PLUGIN_NAME: &str = "arkts";

#[cfg(feature = "napi")]
#[derive(Clone)]
pub struct ExternalLinterCallbacks {
    pub load_plugin: ExternalLinterLoadPluginCb,
    pub setup_rule_configs: ExternalLinterSetupRuleConfigsCb,
    pub lint_file: ExternalLinterLintFileCb,
    pub create_workspace: ExternalLinterCreateWorkspaceCb,
    pub destroy_workspace: ExternalLinterDestroyWorkspaceCb,
}

#[derive(Debug)]
pub struct StandaloneDiagnostic {
    pub rule_name: String,
    pub severity: StandaloneSeverity,
    pub message: String,
    pub start: u32,
    pub end: u32,
}

#[derive(Clone, Debug)]
pub struct StandaloneRuleConfig {
    pub name: String,
    pub severity: StandaloneSeverity,
    pub options: Vec<serde_json::Value>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StandaloneSeverity {
    Warn,
    Error,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ArktsCheck {
    IdentifiersAsPropNames,
    NoSymbol,
    NoPrivateIdentifiers,
    NoVar,
    NoAnyUnknown,
    NoCallSignatures,
    NoCtorSignaturesType,
    NoMultipleStaticBlocks,
    NoIndexedSignatures,
    NoIntersectionTypes,
    NoTypingWithThis,
    NoConditionalTypes,
    NoCtorPropDecls,
    NoCtorSignaturesIface,
    NoAliasesByIndex,
    NoPropsByIndex,
    NoFuncExpressions,
    NoClassLiterals,
    AsCasts,
    NoJsx,
    NoDelete,
    NoTypeQuery,
    NoIn,
    NoDestructAssignment,
    NoCommaOutsideLoops,
    NoDestructDecls,
    NoForIn,
    NoMappedTypes,
    NoWith,
    LimitedThrow,
    NoImplicitReturnTypes,
    NoDestructParams,
    NoNestedFuncs,
    NoStandaloneThis,
    NoGenerators,
    NoIs,
    NoSpread,
    NoCtorSignaturesFuncs,
    NoRequire,
    NoExportAssignment,
    NoAmbientDecls,
    NoModuleWildcards,
    NoUmd,
    NoNewTarget,
    NoDefiniteAssignment,
    NoPrototypeAssignment,
    NoGlobalThis,
    NoUtilityTypes,
    NoFuncApplyCall,
    NoFuncBind,
    NoAsConst,
    NoImportAssertions,
    LimitedStdlib,
    StrictTypingRequired,
    NoMisplacedImports,
    SystemApiVersion,
    Noop,
}

#[derive(Clone, Copy, Debug)]
struct ArktsRule {
    name: &'static str,
    code: Option<&'static str>,
    message: &'static str,
    check: ArktsCheck,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ArktsRuleMeta {
    pub name: &'static str,
    pub code: Option<&'static str>,
    pub message: &'static str,
    pub has_options: bool,
}

static ARKTS_RULES: &[ArktsRule] = &[
    rule(
        "identifiers-as-prop-names",
        "10605001",
        "ArkTS requires object property names to be valid identifiers.",
        ArktsCheck::IdentifiersAsPropNames,
    ),
    rule(
        "no-symbol",
        "10605002",
        "ArkTS does not support Symbol() or the symbol type.",
        ArktsCheck::NoSymbol,
    ),
    rule(
        "no-private-identifiers",
        "10605003",
        "ArkTS does not support private identifiers starting with #. Use the private keyword instead.",
        ArktsCheck::NoPrivateIdentifiers,
    ),
    rule(
        "unique-names",
        "10605004",
        "ArkTS requires unique names for types, namespaces, and values.",
        ArktsCheck::Noop,
    ),
    rule(
        "no-var",
        "10605005",
        "ArkTS does not support var. Use let or const instead.",
        ArktsCheck::NoVar,
    ),
    rule(
        "no-any-unknown",
        "10605008",
        "ArkTS does not support any or unknown. Specify an explicit type.",
        ArktsCheck::NoAnyUnknown,
    ),
    rule(
        "no-call-signatures",
        "10605014",
        "ArkTS does not support call signatures in object types.",
        ArktsCheck::NoCallSignatures,
    ),
    rule(
        "no-ctor-signatures-type",
        "10605015",
        "ArkTS does not support constructor signatures in object types.",
        ArktsCheck::NoCtorSignaturesType,
    ),
    rule(
        "no-multiple-static-blocks",
        "10605016",
        "ArkTS supports only one static block per class.",
        ArktsCheck::NoMultipleStaticBlocks,
    ),
    rule(
        "no-indexed-signatures",
        "10605017",
        "ArkTS does not support index signatures.",
        ArktsCheck::NoIndexedSignatures,
    ),
    rule(
        "no-intersection-types",
        "10605019",
        "ArkTS does not support intersection types. Use inheritance instead.",
        ArktsCheck::NoIntersectionTypes,
    ),
    rule(
        "no-typing-with-this",
        "10605021",
        "ArkTS does not support this in type positions.",
        ArktsCheck::NoTypingWithThis,
    ),
    rule(
        "no-conditional-types",
        "10605022",
        "ArkTS does not support conditional types or infer types.",
        ArktsCheck::NoConditionalTypes,
    ),
    rule(
        "no-ctor-prop-decls",
        "10605025",
        "ArkTS does not support declaring properties in constructor parameters.",
        ArktsCheck::NoCtorPropDecls,
    ),
    rule(
        "no-ctor-signatures-iface",
        "10605027",
        "ArkTS does not support constructor signatures in interfaces.",
        ArktsCheck::NoCtorSignaturesIface,
    ),
    rule(
        "no-aliases-by-index",
        "10605028",
        "ArkTS does not support indexed access types.",
        ArktsCheck::NoAliasesByIndex,
    ),
    rule(
        "no-props-by-index",
        "10605029",
        "ArkTS does not support property access by non-numeric indexes.",
        ArktsCheck::NoPropsByIndex,
    ),
    rule(
        "no-structural-typing",
        "10605030",
        "ArkTS does not support structural typing.",
        ArktsCheck::Noop,
    ),
    rule(
        "no-inferred-generic-params",
        "10605034",
        "ArkTS limits type inference for generic function calls.",
        ArktsCheck::Noop,
    ),
    rule(
        "no-untyped-obj-literals",
        "10605038",
        "ArkTS requires object literals to have inferrable or explicit types.",
        ArktsCheck::Noop,
    ),
    rule(
        "no-obj-literals-as-types",
        "10605040",
        "ArkTS does not support object literal types.",
        ArktsCheck::Noop,
    ),
    rule(
        "no-noninferrable-arr-literals",
        "10605043",
        "ArkTS requires array literal element types to be inferrable.",
        ArktsCheck::Noop,
    ),
    rule(
        "no-func-expressions",
        "10605046",
        "ArkTS does not support function expressions. Use arrow functions instead.",
        ArktsCheck::NoFuncExpressions,
    ),
    rule(
        "no-class-literals",
        "10605050",
        "ArkTS does not support class expressions.",
        ArktsCheck::NoClassLiterals,
    ),
    rule(
        "implements-only-iface",
        "10605051",
        "ArkTS classes may implement interfaces only.",
        ArktsCheck::Noop,
    ),
    rule(
        "no-method-reassignment",
        "10605052",
        "ArkTS does not support method reassignment.",
        ArktsCheck::Noop,
    ),
    rule(
        "as-casts",
        "10605053",
        "ArkTS supports as casts only.",
        ArktsCheck::AsCasts,
    ),
    rule(
        "no-jsx",
        "10605054",
        "ArkTS does not support JSX.",
        ArktsCheck::NoJsx,
    ),
    rule(
        "no-polymorphic-unops",
        "10605055",
        "ArkTS restricts unary operator semantics.",
        ArktsCheck::Noop,
    ),
    rule(
        "no-delete",
        "10605059",
        "ArkTS does not support the delete operator.",
        ArktsCheck::NoDelete,
    ),
    rule(
        "no-type-query",
        "10605060",
        "ArkTS does not support typeof in type positions.",
        ArktsCheck::NoTypeQuery,
    ),
    rule(
        "instanceof-ref-types",
        "10605065",
        "ArkTS restricts instanceof to reference types.",
        ArktsCheck::Noop,
    ),
    rule(
        "no-in",
        "10605066",
        "ArkTS does not support the in operator.",
        ArktsCheck::NoIn,
    ),
    rule(
        "no-destruct-assignment",
        "10605069",
        "ArkTS does not support destructuring assignment.",
        ArktsCheck::NoDestructAssignment,
    ),
    rule(
        "no-comma-outside-loops",
        "10605071",
        "ArkTS restricts comma expressions outside loops.",
        ArktsCheck::NoCommaOutsideLoops,
    ),
    rule(
        "no-destruct-decls",
        "10605074",
        "ArkTS does not support destructuring declarations.",
        ArktsCheck::NoDestructDecls,
    ),
    rule(
        "no-types-in-catch",
        "10605079",
        "ArkTS does not support type annotations in catch clauses.",
        ArktsCheck::Noop,
    ),
    rule(
        "no-for-in",
        "10605080",
        "ArkTS does not support for-in statements.",
        ArktsCheck::NoForIn,
    ),
    rule(
        "no-mapped-types",
        "10605083",
        "ArkTS does not support mapped types.",
        ArktsCheck::NoMappedTypes,
    ),
    rule(
        "no-with",
        "10605084",
        "ArkTS does not support with statements.",
        ArktsCheck::NoWith,
    ),
    rule(
        "limited-throw",
        "10605087",
        "ArkTS restricts thrown values to Error-derived objects.",
        ArktsCheck::LimitedThrow,
    ),
    rule(
        "no-implicit-return-types",
        "10605090",
        "ArkTS requires explicit return types for functions and methods.",
        ArktsCheck::NoImplicitReturnTypes,
    ),
    rule(
        "no-destruct-params",
        "10605091",
        "ArkTS does not support destructuring parameters.",
        ArktsCheck::NoDestructParams,
    ),
    rule(
        "no-nested-funcs",
        "10605092",
        "ArkTS does not support nested function declarations.",
        ArktsCheck::NoNestedFuncs,
    ),
    rule(
        "no-standalone-this",
        "10605093",
        "ArkTS does not support standalone this.",
        ArktsCheck::NoStandaloneThis,
    ),
    rule(
        "no-generators",
        "10605094",
        "ArkTS does not support generator functions.",
        ArktsCheck::NoGenerators,
    ),
    rule(
        "no-is",
        "10605096",
        "ArkTS does not support is type predicates.",
        ArktsCheck::NoIs,
    ),
    rule(
        "no-spread",
        "10605099",
        "ArkTS restricts spread syntax.",
        ArktsCheck::NoSpread,
    ),
    rule(
        "no-extend-same-prop",
        "106050102",
        "ArkTS interfaces cannot extend interfaces with duplicate properties.",
        ArktsCheck::Noop,
    ),
    rule(
        "no-decl-merging",
        "10605103",
        "ArkTS does not support declaration merging.",
        ArktsCheck::Noop,
    ),
    rule(
        "extends-only-class",
        "10605104",
        "ArkTS classes can extend classes only.",
        ArktsCheck::Noop,
    ),
    rule(
        "no-ctor-signatures-funcs",
        "10605106",
        "ArkTS does not support constructor function types.",
        ArktsCheck::NoCtorSignaturesFuncs,
    ),
    rule(
        "no-enum-mixed-types",
        "10605111",
        "ArkTS enum members must be initialized with same-type compile-time expressions.",
        ArktsCheck::Noop,
    ),
    rule(
        "no-enum-merging",
        "10605113",
        "ArkTS does not support enum declaration merging.",
        ArktsCheck::Noop,
    ),
    rule(
        "no-ns-as-obj",
        "10605114",
        "ArkTS does not support using namespaces as objects.",
        ArktsCheck::Noop,
    ),
    rule(
        "no-ns-statements",
        "10605116",
        "ArkTS does not support non-declaration statements in namespaces.",
        ArktsCheck::Noop,
    ),
    rule(
        "no-require",
        "10605121",
        "ArkTS does not support require or import assignment.",
        ArktsCheck::NoRequire,
    ),
    rule(
        "no-export-assignment",
        "10605126",
        "ArkTS does not support export = syntax.",
        ArktsCheck::NoExportAssignment,
    ),
    rule(
        "no-ambient-decls",
        "10605128",
        "ArkTS does not support ambient module declarations.",
        ArktsCheck::NoAmbientDecls,
    ),
    rule(
        "no-module-wildcards",
        "10605129",
        "ArkTS does not support wildcards in module names.",
        ArktsCheck::NoModuleWildcards,
    ),
    rule(
        "no-umd",
        "10605130",
        "ArkTS does not support UMD declarations.",
        ArktsCheck::NoUmd,
    ),
    rule(
        "no-new-target",
        "10605132",
        "ArkTS does not support new.target.",
        ArktsCheck::NoNewTarget,
    ),
    rule(
        "no-definite-assignment",
        "10605134",
        "ArkTS does not support definite assignment assertions.",
        ArktsCheck::NoDefiniteAssignment,
    ),
    rule(
        "no-prototype-assignment",
        "10605136",
        "ArkTS does not support prototype assignment.",
        ArktsCheck::NoPrototypeAssignment,
    ),
    rule(
        "no-globalthis",
        "10605137",
        "ArkTS does not support globalThis.",
        ArktsCheck::NoGlobalThis,
    ),
    rule(
        "no-utility-types",
        "10605138",
        "ArkTS supports only Partial, Required, Readonly, and Record utility types.",
        ArktsCheck::NoUtilityTypes,
    ),
    rule(
        "no-func-props",
        "10605139",
        "ArkTS does not support declaring properties on functions.",
        ArktsCheck::Noop,
    ),
    rule(
        "no-func-apply-call",
        "10605152",
        "ArkTS does not support Function.apply or Function.call.",
        ArktsCheck::NoFuncApplyCall,
    ),
    rule(
        "no-func-bind",
        "10605140",
        "ArkTS does not support Function.bind.",
        ArktsCheck::NoFuncBind,
    ),
    rule(
        "no-as-const",
        "10605142",
        "ArkTS does not support as const assertions.",
        ArktsCheck::NoAsConst,
    ),
    rule(
        "no-import-assertions",
        "10605143",
        "ArkTS does not support import assertions.",
        ArktsCheck::NoImportAssertions,
    ),
    rule(
        "limited-stdlib",
        "10605144",
        "ArkTS restricts dynamic standard library APIs.",
        ArktsCheck::LimitedStdlib,
    ),
    rule(
        "strict-typing-required",
        "10605146",
        "ArkTS does not allow disabling type checking with @ts-ignore or @ts-nocheck.",
        ArktsCheck::StrictTypingRequired,
    ),
    rule(
        "no-ts-deps",
        "10605147",
        "TypeScript and JavaScript files cannot import ETS source files.",
        ArktsCheck::Noop,
    ),
    rule_without_code(
        "no-classes-as-obj",
        "ArkTS does not support using classes as objects.",
        ArktsCheck::Noop,
    ),
    rule_without_code(
        "no-misplaced-imports",
        "ArkTS requires import declarations to appear before other statements.",
        ArktsCheck::NoMisplacedImports,
    ),
    rule_without_code(
        "limited-esobj",
        "ArkTS restricts ESObject usage.",
        ArktsCheck::Noop,
    ),
    rule_without_code(
        "system-api-version",
        "ArkTS system API usage must be supported by the configured minimum API version.",
        ArktsCheck::SystemApiVersion,
    ),
];

const fn rule(
    name: &'static str,
    code: &'static str,
    message: &'static str,
    check: ArktsCheck,
) -> ArktsRule {
    ArktsRule {
        name,
        code: Some(code),
        message,
        check,
    }
}

const fn rule_without_code(
    name: &'static str,
    message: &'static str,
    check: ArktsCheck,
) -> ArktsRule {
    ArktsRule {
        name,
        code: None,
        message,
        check,
    }
}

#[derive(Clone, Debug, Default)]
struct ArktsRuleOptions {
    min_api_version: Option<u32>,
    system_api_versions: Vec<(String, SystemApiVersion)>,
}

#[derive(Clone, Debug, Default)]
struct ArktsOptionStore {
    default: ArktsRuleOptions,
    by_options_id: Vec<ArktsRuleOptions>,
}

impl ArktsOptionStore {
    fn get(&self, options_id: usize) -> &ArktsRuleOptions {
        self.by_options_id.get(options_id).unwrap_or(&self.default)
    }
}

#[cfg(feature = "napi")]
#[derive(Debug)]
struct ExternalState {
    rules: Vec<u32>,
    delegate_options_by_id: Vec<u32>,
}

#[cfg(feature = "napi")]
impl Default for ExternalState {
    fn default() -> Self {
        Self {
            rules: Vec::new(),
            delegate_options_by_id: vec![0],
        }
    }
}

#[cfg(feature = "napi")]
impl ExternalState {
    fn setup_rule_options(&mut self, options_json: &str) -> Result<String, String> {
        let config: ExternalRuleOptionsConfig = serde_json::from_str(options_json)
            .map_err(|err| format!("Failed to parse external plugin options: {err}"))?;

        let option_count = config.options.len().max(1);
        self.delegate_options_by_id = vec![0; option_count];

        let mut delegate_rule_ids = vec![0_u32];
        let mut delegate_options = vec![Vec::<serde_json::Value>::new()];

        for options_id in 1..option_count {
            let Some(rule_id) = config.rule_ids.get(options_id).copied() else {
                continue;
            };
            let options = config.options.get(options_id).cloned().unwrap_or_default();
            let delegate_rule_id = self
                .rules
                .get(rule_id as usize)
                .copied()
                .ok_or_else(|| format!("Unknown external rule id {rule_id}."))?;

            let delegate_options_id = u32::try_from(delegate_options.len())
                .map_err(|_| "JS plugin options id does not fit in u32.".to_string())?;
            self.delegate_options_by_id[options_id] = delegate_options_id;
            delegate_rule_ids.push(delegate_rule_id);
            delegate_options.push(options);
        }

        serde_json::to_string(&serde_json::json!({
            "cwd": config.cwd,
            "workspaceUri": config.workspace_uri,
            "ruleIds": delegate_rule_ids,
            "options": delegate_options,
        }))
        .map_err(|err| format!("Failed to serialize JS plugin options: {err}"))
    }
}

#[cfg(feature = "napi")]
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExternalRuleOptionsConfig {
    cwd: String,
    workspace_uri: Option<String>,
    rule_ids: Vec<u32>,
    options: Vec<Vec<serde_json::Value>>,
}

fn parse_arkts_rule_options(
    rule: &ArktsRule,
    raw_options: &[serde_json::Value],
    cwd: &Path,
    default_options: &ArktsRuleOptions,
) -> Result<ArktsRuleOptions, String> {
    if rule.check != ArktsCheck::SystemApiVersion {
        return Ok(default_options.clone());
    }

    let mut options = default_options.clone();
    let Some(first) = raw_options.first() else {
        return Ok(options);
    };
    let Some(object) = first.as_object() else {
        return Err(
            "arkts/system-api-version expects an options object, for example { \"minApiVersion\": 11 }."
                .to_string(),
        );
    };

    if let Some(min_api_version) = option_u32(object, &["minApiVersion", "minVersion"]) {
        options.min_api_version = Some(min_api_version);
    }

    if let Some(file) = object
        .get("apiVersionFile")
        .or_else(|| object.get("apisFile"))
        .and_then(serde_json::Value::as_str)
    {
        let file_path = resolve_config_path(cwd, file);
        merge_api_version_file(&mut options, &file_path)?;
    }

    if let Some(apis) = object
        .get("apis")
        .or_else(|| object.get("apiVersions"))
        .and_then(serde_json::Value::as_object)
    {
        merge_api_version_map(&mut options, apis)?;
    }

    Ok(options)
}

fn option_u32(object: &serde_json::Map<String, serde_json::Value>, keys: &[&str]) -> Option<u32> {
    keys.iter().find_map(|key| {
        object.get(*key).and_then(|value| {
            value
                .as_u64()
                .and_then(|value| u32::try_from(value).ok())
                .or_else(|| value.as_str().and_then(|value| value.parse::<u32>().ok()))
        })
    })
}

fn resolve_config_path(cwd: &Path, path: &str) -> PathBuf {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    }
}

fn merge_api_version_file(options: &mut ArktsRuleOptions, path: &Path) -> Result<(), String> {
    let mut source = fs::read_to_string(path).map_err(|err| {
        format!(
            "Failed to read ArkTS system API version file `{}`: {err}",
            path.display()
        )
    })?;
    if matches!(path.extension().and_then(|ext| ext.to_str()), Some("jsonc")) {
        json_strip_comments::strip(&mut source).map_err(|err| {
            format!(
                "Failed to strip comments from ArkTS system API version file `{}`: {err}",
                path.display()
            )
        })?;
    }

    let value: serde_json::Value = serde_json::from_str(&source).map_err(|err| {
        format!(
            "Failed to parse ArkTS system API version file `{}`: {err}",
            path.display()
        )
    })?;
    let object = value
        .as_object()
        .ok_or_else(|| "ArkTS system API version file must contain a JSON object.".to_string())?;

    let apis = object
        .get("apis")
        .or_else(|| object.get("apiVersions"))
        .and_then(serde_json::Value::as_object)
        .unwrap_or(object);
    merge_api_version_map(options, apis)
}

fn merge_api_version_map(
    options: &mut ArktsRuleOptions,
    apis: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), String> {
    for (api, value) in apis {
        let version = parse_system_api_version_value(api, value)?;
        options.system_api_versions.push((api.clone(), version));
    }
    Ok(())
}

fn parse_system_api_version_value(
    api: &str,
    value: &serde_json::Value,
) -> Result<SystemApiVersion, String> {
    if let Some(since) = version_u32(value) {
        return Ok(SystemApiVersion {
            since,
            removed: None,
        });
    }

    let Some(object) = value.as_object() else {
        return Err(format!(
            "ArkTS system API version for `{api}` must be an integer or an object with `since`."
        ));
    };

    let Some(since) = object
        .get("since")
        .or_else(|| object.get("version"))
        .or_else(|| object.get("apiVersion"))
        .or_else(|| object.get("apiLevel"))
        .and_then(version_u32)
    else {
        return Err(format!(
            "ArkTS system API version object for `{api}` must include integer `since`."
        ));
    };

    let removed = object
        .get("removed")
        .or_else(|| object.get("removedVersion"))
        .or_else(|| object.get("deleteVersion"))
        .or_else(|| object.get("deletedVersion"))
        .or_else(|| object.get("removalVersion"))
        .or_else(|| object.get("deprecatedSince"))
        .or_else(|| object.get("deprecatedVersion"))
        .and_then(version_u32);

    if let Some(removed) = removed
        && removed < since
    {
        return Err(format!(
            "ArkTS system API `{api}` removal version must be greater than or equal to since version."
        ));
    }

    Ok(SystemApiVersion { since, removed })
}

fn version_u32(value: &serde_json::Value) -> Option<u32> {
    value
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .or_else(|| value.as_str().and_then(|value| value.parse::<u32>().ok()))
}

fn find_project_min_api_version(cwd: &Path) -> Option<u32> {
    [
        "AppScope/app.json5",
        "app.json5",
        "src/main/module.json5",
        "entry/src/main/module.json5",
    ]
    .into_iter()
    .filter_map(|path| fs::read_to_string(cwd.join(path)).ok())
    .find_map(|source| {
        find_numeric_property(&source, "minAPIVersion")
            .or_else(|| find_numeric_property(&source, "minApiVersion"))
    })
}

fn find_numeric_property(source: &str, key: &str) -> Option<u32> {
    let double_quoted = format!("\"{key}\"");
    let single_quoted = format!("'{key}'");
    let start = source
        .find(&double_quoted)
        .or_else(|| source.find(&single_quoted))
        .or_else(|| source.find(key))?;
    let after_key = &source[start..];
    let colon = after_key.find(':')?;
    let mut value = after_key[colon + 1..].trim_start();
    if let Some(stripped) = value.strip_prefix('"').or_else(|| value.strip_prefix('\'')) {
        value = stripped;
    }
    let digits_len = value
        .chars()
        .take_while(|char| char.is_ascii_digit())
        .map(char::len_utf8)
        .sum();
    if digits_len == 0 {
        return None;
    }
    value[..digits_len].parse().ok()
}

#[cfg(feature = "napi")]
pub fn create_external_linter(delegate: Option<ExternalLinterCallbacks>) -> ExternalLinter {
    let state = Arc::new(Mutex::new(ExternalState::default()));

    ExternalLinter::new(
        load_plugin_callback(Arc::clone(&state), delegate.clone()),
        setup_rule_configs_callback(Arc::clone(&state), delegate.clone()),
        lint_file_callback(Arc::clone(&state), delegate.clone()),
        create_workspace_callback(delegate.clone()),
        destroy_workspace_callback(delegate),
    )
}

#[cfg(feature = "napi")]
fn load_plugin_callback(
    state: Arc<Mutex<ExternalState>>,
    delegate: Option<ExternalLinterCallbacks>,
) -> ExternalLinterLoadPluginCb {
    Arc::new(Box::new(
        move |plugin_url, plugin_name, plugin_name_is_alias, workspace_uri| {
            let Some(delegate) = &delegate else {
                return Err(
                    "JavaScript plugins are not available in the cargo lint runner.".to_string(),
                );
            };

            let mut result = (delegate.load_plugin)(
                plugin_url,
                plugin_name,
                plugin_name_is_alias,
                workspace_uri,
            )?;
            let mut state = state.lock().map_err(|err| err.to_string())?;
            let offset = state.rules.len();
            let delegate_offset = u32::try_from(result.offset)
                .map_err(|_| "JS plugin rule offset does not fit in u32.".to_string())?;
            state
                .rules
                .extend((0..result.rule_names.len()).map(|index| delegate_offset + index as u32));
            result.offset = offset;
            Ok(result)
        },
    ))
}

#[cfg(feature = "napi")]
fn setup_rule_configs_callback(
    state: Arc<Mutex<ExternalState>>,
    delegate: Option<ExternalLinterCallbacks>,
) -> ExternalLinterSetupRuleConfigsCb {
    Arc::new(Box::new(move |options_json| {
        let delegate_options_json = {
            let mut state = state.lock().map_err(|err| err.to_string())?;
            state.setup_rule_options(&options_json)?
        };

        if let Some(delegate) = &delegate {
            (delegate.setup_rule_configs)(delegate_options_json)?;
        }
        Ok(())
    }))
}

#[cfg(feature = "napi")]
fn create_workspace_callback(
    delegate: Option<ExternalLinterCallbacks>,
) -> ExternalLinterCreateWorkspaceCb {
    Arc::new(Box::new(move |workspace_uri| {
        if let Some(delegate) = &delegate {
            (delegate.create_workspace)(workspace_uri)
        } else {
            Ok(())
        }
    }))
}

#[cfg(feature = "napi")]
fn destroy_workspace_callback(
    delegate: Option<ExternalLinterCallbacks>,
) -> ExternalLinterDestroyWorkspaceCb {
    Arc::new(Box::new(move |workspace_uri| {
        if let Some(delegate) = &delegate {
            (delegate.destroy_workspace)(workspace_uri)
        } else {
            Ok(())
        }
    }))
}

#[cfg(feature = "napi")]
fn lint_file_callback(
    state: Arc<Mutex<ExternalState>>,
    delegate: Option<ExternalLinterCallbacks>,
) -> ExternalLinterLintFileCb {
    Arc::new(Box::new(
        move |file_path,
              rule_ids,
              options_ids,
              settings_json,
              globals_json,
              workspace_uri,
              allocator| {
            let (delegate_rules, delegate_options, delegate_rule_indices) = {
                let state = state.lock().map_err(|err| err.to_string())?;
                split_rules(&state, &rule_ids, &options_ids)?
            };

            let mut diagnostics = Vec::new();
            if !delegate_rules.is_empty() {
                let Some(delegate) = &delegate else {
                    return Err(
                        "JavaScript plugins are not available in this lint runner.".to_string()
                    );
                };

                let mut delegate_diagnostics = (delegate.lint_file)(
                    file_path,
                    delegate_rules,
                    delegate_options,
                    settings_json,
                    globals_json,
                    workspace_uri,
                    allocator,
                )?;

                for diagnostic in &mut delegate_diagnostics {
                    let mapped = delegate_rule_indices
                        .get(diagnostic.rule_index as usize)
                        .copied()
                        .ok_or_else(|| {
                            format!(
                                "JS plugin returned invalid rule index {}.",
                                diagnostic.rule_index
                            )
                        })?;
                    diagnostic.rule_index = mapped;
                }
                diagnostics.extend(delegate_diagnostics);
            }

            Ok(diagnostics)
        },
    ))
}

#[cfg(feature = "napi")]
type SplitRules = (Vec<u32>, Vec<u32>, Vec<u32>);

#[cfg(feature = "napi")]
fn split_rules(
    state: &ExternalState,
    rule_ids: &[u32],
    options_ids: &[u32],
) -> Result<SplitRules, String> {
    let mut delegate_rules = Vec::new();
    let mut delegate_options = Vec::new();
    let mut delegate_rule_indices = Vec::new();

    for (active_index, rule_id) in rule_ids.iter().enumerate() {
        let delegate_rule_id = state
            .rules
            .get(*rule_id as usize)
            .copied()
            .ok_or_else(|| format!("Unknown external rule id {rule_id}."))?;
        delegate_rules.push(delegate_rule_id);
        let options_id = options_ids.get(active_index).copied().unwrap_or(0) as usize;
        delegate_options.push(
            state
                .delegate_options_by_id
                .get(options_id)
                .copied()
                .unwrap_or(0),
        );
        delegate_rule_indices.push(active_index as u32);
    }

    Ok((delegate_rules, delegate_options, delegate_rule_indices))
}

#[derive(Clone, Copy)]
struct ActiveArktsRule {
    active_index: u32,
    rule: &'static ArktsRule,
    options_id: usize,
}

fn run_arkts_rules(
    file_path: &str,
    source_text: &str,
    allocator: &Allocator,
    active_rules: &[ActiveArktsRule],
    options: ArktsOptionStore,
    source_type: Option<SourceType>,
) -> Result<Vec<LintFileResult>, String> {
    let source_type = source_type.unwrap_or_else(|| source_type_for_arkts_path(file_path));
    let parser_return = Parser::new(allocator, source_text, source_type).parse();

    let mut visitor = ArktsVisitor {
        active: ActiveArktsRules::from_active(active_rules),
        diagnostics: Vec::new(),
        span_converter: None,
        options,
        function_depth: 0,
        system_api_imports: HashMap::new(),
        reported_system_api_config: false,
    };
    visitor.visit_program(&parser_return.program);
    Ok(visitor.diagnostics)
}

fn source_type_for_arkts_path(file_path: &str) -> SourceType {
    match Path::new(file_path)
        .extension()
        .and_then(|ext| ext.to_str())
    {
        Some("ets") => SourceType::ets(),
        _ => SourceType::from_path(file_path).unwrap_or_else(|_| SourceType::ts()),
    }
}

fn is_arkts_file(path: &Path) -> bool {
    matches!(path.extension().and_then(|ext| ext.to_str()), Some("ets"))
}

#[derive(Default)]
struct ActiveArktsRules {
    identifiers_as_prop_names: Option<ActiveArktsRule>,
    no_symbol: Option<ActiveArktsRule>,
    no_private_identifiers: Option<ActiveArktsRule>,
    no_var: Option<ActiveArktsRule>,
    no_any_unknown: Option<ActiveArktsRule>,
    no_call_signatures: Option<ActiveArktsRule>,
    no_ctor_signatures_type: Option<ActiveArktsRule>,
    no_multiple_static_blocks: Option<ActiveArktsRule>,
    no_indexed_signatures: Option<ActiveArktsRule>,
    no_intersection_types: Option<ActiveArktsRule>,
    no_typing_with_this: Option<ActiveArktsRule>,
    no_conditional_types: Option<ActiveArktsRule>,
    no_ctor_prop_decls: Option<ActiveArktsRule>,
    no_ctor_signatures_iface: Option<ActiveArktsRule>,
    no_aliases_by_index: Option<ActiveArktsRule>,
    no_props_by_index: Option<ActiveArktsRule>,
    no_func_expressions: Option<ActiveArktsRule>,
    no_class_literals: Option<ActiveArktsRule>,
    as_casts: Option<ActiveArktsRule>,
    no_jsx: Option<ActiveArktsRule>,
    no_delete: Option<ActiveArktsRule>,
    no_type_query: Option<ActiveArktsRule>,
    no_in: Option<ActiveArktsRule>,
    no_destruct_assignment: Option<ActiveArktsRule>,
    no_comma_outside_loops: Option<ActiveArktsRule>,
    no_destruct_decls: Option<ActiveArktsRule>,
    no_for_in: Option<ActiveArktsRule>,
    no_mapped_types: Option<ActiveArktsRule>,
    no_with: Option<ActiveArktsRule>,
    limited_throw: Option<ActiveArktsRule>,
    no_implicit_return_types: Option<ActiveArktsRule>,
    no_destruct_params: Option<ActiveArktsRule>,
    no_nested_funcs: Option<ActiveArktsRule>,
    no_standalone_this: Option<ActiveArktsRule>,
    no_generators: Option<ActiveArktsRule>,
    no_is: Option<ActiveArktsRule>,
    no_spread: Option<ActiveArktsRule>,
    no_ctor_signatures_funcs: Option<ActiveArktsRule>,
    no_require: Option<ActiveArktsRule>,
    no_export_assignment: Option<ActiveArktsRule>,
    no_ambient_decls: Option<ActiveArktsRule>,
    no_module_wildcards: Option<ActiveArktsRule>,
    no_umd: Option<ActiveArktsRule>,
    no_new_target: Option<ActiveArktsRule>,
    no_definite_assignment: Option<ActiveArktsRule>,
    no_prototype_assignment: Option<ActiveArktsRule>,
    no_globalthis: Option<ActiveArktsRule>,
    no_utility_types: Option<ActiveArktsRule>,
    no_func_apply_call: Option<ActiveArktsRule>,
    no_func_bind: Option<ActiveArktsRule>,
    no_as_const: Option<ActiveArktsRule>,
    no_import_assertions: Option<ActiveArktsRule>,
    limited_stdlib: Option<ActiveArktsRule>,
    strict_typing_required: Option<ActiveArktsRule>,
    no_misplaced_imports: Option<ActiveArktsRule>,
    system_api_version: Option<ActiveArktsRule>,
}

impl ActiveArktsRules {
    fn from_active(active_rules: &[ActiveArktsRule]) -> Self {
        let mut active = Self::default();
        for rule in active_rules {
            match rule.rule.check {
                ArktsCheck::IdentifiersAsPropNames => {
                    active.identifiers_as_prop_names = Some(*rule)
                }
                ArktsCheck::NoSymbol => active.no_symbol = Some(*rule),
                ArktsCheck::NoPrivateIdentifiers => active.no_private_identifiers = Some(*rule),
                ArktsCheck::NoVar => active.no_var = Some(*rule),
                ArktsCheck::NoAnyUnknown => active.no_any_unknown = Some(*rule),
                ArktsCheck::NoCallSignatures => active.no_call_signatures = Some(*rule),
                ArktsCheck::NoCtorSignaturesType => active.no_ctor_signatures_type = Some(*rule),
                ArktsCheck::NoMultipleStaticBlocks => {
                    active.no_multiple_static_blocks = Some(*rule)
                }
                ArktsCheck::NoIndexedSignatures => active.no_indexed_signatures = Some(*rule),
                ArktsCheck::NoIntersectionTypes => active.no_intersection_types = Some(*rule),
                ArktsCheck::NoTypingWithThis => active.no_typing_with_this = Some(*rule),
                ArktsCheck::NoConditionalTypes => active.no_conditional_types = Some(*rule),
                ArktsCheck::NoCtorPropDecls => active.no_ctor_prop_decls = Some(*rule),
                ArktsCheck::NoCtorSignaturesIface => active.no_ctor_signatures_iface = Some(*rule),
                ArktsCheck::NoAliasesByIndex => active.no_aliases_by_index = Some(*rule),
                ArktsCheck::NoPropsByIndex => active.no_props_by_index = Some(*rule),
                ArktsCheck::NoFuncExpressions => active.no_func_expressions = Some(*rule),
                ArktsCheck::NoClassLiterals => active.no_class_literals = Some(*rule),
                ArktsCheck::AsCasts => active.as_casts = Some(*rule),
                ArktsCheck::NoJsx => active.no_jsx = Some(*rule),
                ArktsCheck::NoDelete => active.no_delete = Some(*rule),
                ArktsCheck::NoTypeQuery => active.no_type_query = Some(*rule),
                ArktsCheck::NoIn => active.no_in = Some(*rule),
                ArktsCheck::NoDestructAssignment => active.no_destruct_assignment = Some(*rule),
                ArktsCheck::NoCommaOutsideLoops => active.no_comma_outside_loops = Some(*rule),
                ArktsCheck::NoDestructDecls => active.no_destruct_decls = Some(*rule),
                ArktsCheck::NoForIn => active.no_for_in = Some(*rule),
                ArktsCheck::NoMappedTypes => active.no_mapped_types = Some(*rule),
                ArktsCheck::NoWith => active.no_with = Some(*rule),
                ArktsCheck::LimitedThrow => active.limited_throw = Some(*rule),
                ArktsCheck::NoImplicitReturnTypes => active.no_implicit_return_types = Some(*rule),
                ArktsCheck::NoDestructParams => active.no_destruct_params = Some(*rule),
                ArktsCheck::NoNestedFuncs => active.no_nested_funcs = Some(*rule),
                ArktsCheck::NoStandaloneThis => active.no_standalone_this = Some(*rule),
                ArktsCheck::NoGenerators => active.no_generators = Some(*rule),
                ArktsCheck::NoIs => active.no_is = Some(*rule),
                ArktsCheck::NoSpread => active.no_spread = Some(*rule),
                ArktsCheck::NoCtorSignaturesFuncs => active.no_ctor_signatures_funcs = Some(*rule),
                ArktsCheck::NoRequire => active.no_require = Some(*rule),
                ArktsCheck::NoExportAssignment => active.no_export_assignment = Some(*rule),
                ArktsCheck::NoAmbientDecls => active.no_ambient_decls = Some(*rule),
                ArktsCheck::NoModuleWildcards => active.no_module_wildcards = Some(*rule),
                ArktsCheck::NoUmd => active.no_umd = Some(*rule),
                ArktsCheck::NoNewTarget => active.no_new_target = Some(*rule),
                ArktsCheck::NoDefiniteAssignment => active.no_definite_assignment = Some(*rule),
                ArktsCheck::NoPrototypeAssignment => active.no_prototype_assignment = Some(*rule),
                ArktsCheck::NoGlobalThis => active.no_globalthis = Some(*rule),
                ArktsCheck::NoUtilityTypes => active.no_utility_types = Some(*rule),
                ArktsCheck::NoFuncApplyCall => active.no_func_apply_call = Some(*rule),
                ArktsCheck::NoFuncBind => active.no_func_bind = Some(*rule),
                ArktsCheck::NoAsConst => active.no_as_const = Some(*rule),
                ArktsCheck::NoImportAssertions => active.no_import_assertions = Some(*rule),
                ArktsCheck::LimitedStdlib => active.limited_stdlib = Some(*rule),
                ArktsCheck::StrictTypingRequired => active.strict_typing_required = Some(*rule),
                ArktsCheck::NoMisplacedImports => active.no_misplaced_imports = Some(*rule),
                ArktsCheck::SystemApiVersion => active.system_api_version = Some(*rule),
                ArktsCheck::Noop => {}
            }
        }
        active
    }
}

#[derive(Clone, Debug)]
struct SystemImport {
    module: String,
    imported: Option<String>,
}

struct ArktsVisitor<'c> {
    active: ActiveArktsRules,
    diagnostics: Vec<LintFileResult>,
    span_converter: Option<Utf8ToUtf16Converter<'c>>,
    options: ArktsOptionStore,
    function_depth: usize,
    system_api_imports: HashMap<String, SystemImport>,
    reported_system_api_config: bool,
}

impl ArktsVisitor<'_> {
    fn report(&mut self, active: ActiveArktsRule, span: Span) {
        let message = if let Some(code) = active.rule.code {
            format!(
                "{} ({}: {code})",
                active.rule.message,
                active.rule.doc_name()
            )
        } else {
            format!("{} ({})", active.rule.message, active.rule.doc_name())
        };
        self.report_message(active, span, message);
    }

    fn report_message(&mut self, active: ActiveArktsRule, span: Span, message: String) {
        let mut span = span;
        if let Some(converter) = &mut self.span_converter {
            converter.convert_span(&mut span);
        }

        self.diagnostics.push(LintFileResult {
            rule_index: active.active_index,
            message,
            start: span.start,
            end: span.end,
            fixes: None,
            suggestions: None,
        });
    }

    fn options_for(&self, active: ActiveArktsRule) -> &ArktsRuleOptions {
        self.options.get(active.options_id)
    }

    fn report_system_api_version(&mut self, active: ActiveArktsRule, api: &str, span: Span) {
        let options = self.options_for(active);
        let Some(min_api_version) = options.min_api_version else {
            if !self.reported_system_api_config {
                self.reported_system_api_config = true;
                self.report_message(
                    active,
                    Span::new(0, 0),
                    format!(
                        "arkts/system-api-version requires `minApiVersion` or `minAPIVersion` in project config. ({})",
                        active.rule.doc_name()
                    ),
                );
            }
            return;
        };

        let Some(api_version) = system_api_version(options, api) else {
            return;
        };

        if api_version.since > min_api_version {
            self.report_message(
                active,
                span,
                format!(
                    "System API `{api}` requires API version {}, but the configured minimum supported API version is {min_api_version}. ({})",
                    api_version.since,
                    active.rule.doc_name()
                ),
            );
            return;
        }

        if let Some(removed) = api_version.removed
            && min_api_version >= removed
        {
            self.report_message(
                active,
                span,
                format!(
                    "System API `{api}` was removed or deprecated in API version {removed}, but the configured minimum supported API version is {min_api_version}. ({})",
                    active.rule.doc_name()
                ),
            );
        }
    }

    fn system_api_key_from_expression(
        &self,
        expression: &Expression<'_>,
    ) -> Option<(String, Span)> {
        let mut segments: Vec<&str> = Vec::new();
        let mut current = expression;

        loop {
            match current {
                Expression::Identifier(identifier) => {
                    let import = self.system_api_imports.get(identifier.name.as_str())?;
                    let mut api = import.module.clone();
                    if let Some(imported) = &import.imported {
                        api.push('.');
                        api.push_str(imported);
                    }
                    for segment in segments.iter().rev() {
                        api.push('.');
                        api.push_str(segment);
                    }
                    return Some((api, expression.span()));
                }
                Expression::StaticMemberExpression(member) => {
                    segments.push(member.property.name.as_str());
                    current = &member.object;
                }
                _ => return None,
            }
        }
    }

    fn system_api_key_from_static_member(
        &self,
        member: &StaticMemberExpression<'_>,
    ) -> Option<(String, Span)> {
        let mut segments = vec![member.property.name.as_str()];
        let mut current = &member.object;

        loop {
            match current {
                Expression::Identifier(identifier) => {
                    let import = self.system_api_imports.get(identifier.name.as_str())?;
                    let mut api = import.module.clone();
                    if let Some(imported) = &import.imported {
                        api.push('.');
                        api.push_str(imported);
                    }
                    for segment in segments.iter().rev() {
                        api.push('.');
                        api.push_str(segment);
                    }
                    return Some((api, member.span));
                }
                Expression::StaticMemberExpression(inner) => {
                    segments.push(inner.property.name.as_str());
                    current = &inner.object;
                }
                _ => return None,
            }
        }
    }
}

impl ArktsRule {
    fn doc_name(&self) -> String {
        format!("arkts-{}", self.name)
    }
}

impl<'a> Visit<'a> for ArktsVisitor<'_> {
    fn visit_program(&mut self, it: &Program<'a>) {
        if self.active.system_api_version.is_some() {
            self.system_api_imports = collect_system_api_imports(&it.body);
        }

        if let Some(active) = self.active.strict_typing_required
            && (it.source_text.contains("@ts-ignore") || it.source_text.contains("@ts-nocheck"))
        {
            self.report(active, Span::new(0, 0));
        }

        if let Some(active) = self.active.no_misplaced_imports {
            let mut seen_non_import = false;
            for statement in &it.body {
                let is_import = matches!(
                    statement,
                    Statement::ImportDeclaration(_) | Statement::LazyImportDeclaration(_)
                );
                if is_import && seen_non_import {
                    self.report(active, statement.span());
                } else if !is_import {
                    seen_non_import = true;
                }
            }
        }

        walk::walk_program(self, it);
    }

    fn visit_variable_declaration(&mut self, it: &VariableDeclaration<'a>) {
        if it.kind == VariableDeclarationKind::Var
            && let Some(active) = self.active.no_var
        {
            self.report(active, it.span);
        }
        walk::walk_variable_declaration(self, it);
    }

    fn visit_variable_declarator(&mut self, it: &VariableDeclarator<'a>) {
        if it.definite
            && let Some(active) = self.active.no_definite_assignment
        {
            self.report(active, it.span);
        }
        if matches!(
            it.id,
            BindingPattern::ObjectPattern(_) | BindingPattern::ArrayPattern(_)
        ) && let Some(active) = self.active.no_destruct_decls
        {
            self.report(active, it.id.span());
        }
        walk::walk_variable_declarator(self, it);
    }

    fn visit_identifier_reference(&mut self, it: &IdentifierReference<'a>) {
        if it.name == "globalThis"
            && let Some(active) = self.active.no_globalthis
        {
            self.report(active, it.span);
        }
        walk::walk_identifier_reference(self, it);
    }

    fn visit_private_identifier(&mut self, it: &PrivateIdentifier<'a>) {
        if let Some(active) = self.active.no_private_identifiers {
            self.report(active, it.span);
        }
        walk::walk_private_identifier(self, it);
    }

    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        if is_identifier(&it.callee, "Symbol")
            && let Some(active) = self.active.no_symbol
        {
            self.report(active, it.callee.span());
        }
        if let Some(active) = self.active.system_api_version
            && let Expression::Identifier(_) = &it.callee
            && let Some((api, span)) = self.system_api_key_from_expression(&it.callee)
        {
            self.report_system_api_version(active, &api, span);
        }
        if is_identifier(&it.callee, "require")
            && let Some(active) = self.active.no_require
        {
            self.report(active, it.span);
        }
        if let Some(property_name) = static_member_property(&it.callee) {
            if matches!(property_name, "apply" | "call")
                && let Some(active) = self.active.no_func_apply_call
            {
                self.report(active, it.callee.span());
            }
            if property_name == "bind"
                && let Some(active) = self.active.no_func_bind
            {
                self.report(active, it.callee.span());
            }
            if is_limited_stdlib_call(&it.callee, property_name)
                && let Some(active) = self.active.limited_stdlib
            {
                self.report(active, it.callee.span());
            }
        }
        walk::walk_call_expression(self, it);
    }

    fn visit_new_expression(&mut self, it: &NewExpression<'a>) {
        if is_identifier(&it.callee, "Symbol")
            && let Some(active) = self.active.no_symbol
        {
            self.report(active, it.callee.span());
        }
        walk::walk_new_expression(self, it);
    }

    fn visit_expression(&mut self, it: &Expression<'a>) {
        match it {
            Expression::FunctionExpression(function) => {
                if let Some(active) = self.active.no_func_expressions {
                    self.report(active, function.span);
                }
            }
            Expression::ClassExpression(class) => {
                if let Some(active) = self.active.no_class_literals {
                    self.report(active, class.span);
                }
            }
            Expression::TSAsExpression(expr) => {
                if let Some(active) = self.active.no_as_const
                    && is_const_type_reference(&expr.type_annotation)
                {
                    self.report(active, expr.type_annotation.span());
                }
            }
            Expression::TSTypeAssertion(assertion) => {
                if let Some(active) = self.active.as_casts {
                    self.report(active, assertion.span);
                }
            }
            _ => {}
        }
        walk::walk_expression(self, it);
    }

    fn visit_unary_expression(&mut self, it: &UnaryExpression<'a>) {
        if it.operator == UnaryOperator::Delete
            && let Some(active) = self.active.no_delete
        {
            self.report(active, it.span);
        }
        walk::walk_unary_expression(self, it);
    }

    fn visit_binary_expression(&mut self, it: &BinaryExpression<'a>) {
        if it.operator == BinaryOperator::In
            && let Some(active) = self.active.no_in
        {
            self.report(active, it.span);
        }
        walk::walk_binary_expression(self, it);
    }

    fn visit_private_in_expression(&mut self, it: &PrivateInExpression<'a>) {
        if let Some(active) = self.active.no_in {
            self.report(active, it.span);
        }
        walk::walk_private_in_expression(self, it);
    }

    fn visit_assignment_expression(&mut self, it: &AssignmentExpression<'a>) {
        if matches!(
            it.left,
            AssignmentTarget::ArrayAssignmentTarget(_)
                | AssignmentTarget::ObjectAssignmentTarget(_)
        ) && let Some(active) = self.active.no_destruct_assignment
        {
            self.report(active, it.left.span());
        }
        if is_prototype_assignment_target(&it.left)
            && let Some(active) = self.active.no_prototype_assignment
        {
            self.report(active, it.left.span());
        }
        walk::walk_assignment_expression(self, it);
    }

    fn visit_for_in_statement(&mut self, it: &ForInStatement<'a>) {
        if let Some(active) = self.active.no_for_in {
            self.report(active, it.span);
        }
        walk::walk_for_in_statement(self, it);
    }

    fn visit_with_statement(&mut self, it: &WithStatement<'a>) {
        if let Some(active) = self.active.no_with {
            self.report(active, it.span);
        }
        walk::walk_with_statement(self, it);
    }

    fn visit_throw_statement(&mut self, it: &ThrowStatement<'a>) {
        if let Some(active) = self.active.limited_throw
            && !is_new_error_expression(&it.argument)
        {
            self.report(active, it.argument.span());
        }
        walk::walk_throw_statement(self, it);
    }

    fn visit_function(&mut self, it: &Function<'a>, flags: oxc_syntax::scope::ScopeFlags) {
        if it.generator
            && let Some(active) = self.active.no_generators
        {
            self.report(active, it.span);
        }
        if self.function_depth > 0
            && it.r#type == FunctionType::FunctionDeclaration
            && let Some(active) = self.active.no_nested_funcs
        {
            self.report(active, it.span);
        }
        if it.return_type.is_none()
            && it.body.is_some()
            && !matches!(it.r#type, FunctionType::TSDeclareFunction)
            && let Some(active) = self.active.no_implicit_return_types
        {
            self.report(active, it.span);
        }

        self.function_depth += 1;
        walk::walk_function(self, it, flags);
        self.function_depth -= 1;
    }

    fn visit_this_expression(&mut self, it: &ThisExpression) {
        if self.function_depth == 0
            && let Some(active) = self.active.no_standalone_this
        {
            self.report(active, it.span);
        }
        walk::walk_this_expression(self, it);
    }

    fn visit_formal_parameter(&mut self, it: &FormalParameter<'a>) {
        if matches!(
            it.pattern,
            BindingPattern::ObjectPattern(_) | BindingPattern::ArrayPattern(_)
        ) && let Some(active) = self.active.no_destruct_params
        {
            self.report(active, it.pattern.span());
        }
        if it.accessibility.is_some()
            && let Some(active) = self.active.no_ctor_prop_decls
        {
            self.report(active, it.span);
        }
        walk::walk_formal_parameter(self, it);
    }

    fn visit_class_body(&mut self, it: &ClassBody<'a>) {
        if let Some(active) = self.active.no_multiple_static_blocks {
            let mut seen_static_block = false;
            for element in &it.body {
                if let ClassElement::StaticBlock(block) = element {
                    if seen_static_block {
                        self.report(active, block.span);
                    }
                    seen_static_block = true;
                }
            }
        }
        walk::walk_class_body(self, it);
    }

    fn visit_property_definition(&mut self, it: &PropertyDefinition<'a>) {
        if it.definite
            && let Some(active) = self.active.no_definite_assignment
        {
            self.report(active, it.span);
        }
        walk::walk_property_definition(self, it);
    }

    fn visit_import_declaration(&mut self, it: &ImportDeclaration<'a>) {
        if let Some(active) = self.active.system_api_version
            && let Some(module) = system_module_name(it.source.value.as_str())
        {
            self.report_system_api_version(active, module, it.source.span);
        }
        if it.with_clause.is_some()
            && let Some(active) = self.active.no_import_assertions
        {
            self.report(active, it.span);
        }
        walk::walk_import_declaration(self, it);
    }

    fn visit_export_named_declaration(&mut self, it: &ExportNamedDeclaration<'a>) {
        if it.with_clause.is_some()
            && let Some(active) = self.active.no_import_assertions
        {
            self.report(active, it.span);
        }
        walk::walk_export_named_declaration(self, it);
    }

    fn visit_export_all_declaration(&mut self, it: &ExportAllDeclaration<'a>) {
        if it.with_clause.is_some()
            && let Some(active) = self.active.no_import_assertions
        {
            self.report(active, it.span);
        }
        walk::walk_export_all_declaration(self, it);
    }

    fn visit_jsx_element(&mut self, it: &JSXElement<'a>) {
        if let Some(active) = self.active.no_jsx {
            self.report(active, it.span);
        }
        walk::walk_jsx_element(self, it);
    }

    fn visit_jsx_fragment(&mut self, it: &JSXFragment<'a>) {
        if let Some(active) = self.active.no_jsx {
            self.report(active, it.span);
        }
        walk::walk_jsx_fragment(self, it);
    }

    fn visit_ts_any_keyword(&mut self, it: &TSAnyKeyword) {
        if let Some(active) = self.active.no_any_unknown {
            self.report(active, it.span);
        }
        walk::walk_ts_any_keyword(self, it);
    }

    fn visit_ts_unknown_keyword(&mut self, it: &TSUnknownKeyword) {
        if let Some(active) = self.active.no_any_unknown {
            self.report(active, it.span);
        }
        walk::walk_ts_unknown_keyword(self, it);
    }

    fn visit_ts_symbol_keyword(&mut self, it: &TSSymbolKeyword) {
        if let Some(active) = self.active.no_symbol {
            self.report(active, it.span);
        }
        walk::walk_ts_symbol_keyword(self, it);
    }

    fn visit_ts_call_signature_declaration(&mut self, it: &TSCallSignatureDeclaration<'a>) {
        if let Some(active) = self.active.no_call_signatures {
            self.report(active, it.span);
        }
        walk::walk_ts_call_signature_declaration(self, it);
    }

    fn visit_ts_construct_signature_declaration(
        &mut self,
        it: &TSConstructSignatureDeclaration<'a>,
    ) {
        if let Some(active) = self
            .active
            .no_ctor_signatures_iface
            .or(self.active.no_ctor_signatures_type)
        {
            self.report(active, it.span);
        }
        walk::walk_ts_construct_signature_declaration(self, it);
    }

    fn visit_ts_index_signature(&mut self, it: &TSIndexSignature<'a>) {
        if let Some(active) = self.active.no_indexed_signatures {
            self.report(active, it.span);
        }
        walk::walk_ts_index_signature(self, it);
    }

    fn visit_ts_intersection_type(&mut self, it: &TSIntersectionType<'a>) {
        if let Some(active) = self.active.no_intersection_types {
            self.report(active, it.span);
        }
        walk::walk_ts_intersection_type(self, it);
    }

    fn visit_ts_this_type(&mut self, it: &TSThisType) {
        if let Some(active) = self.active.no_typing_with_this {
            self.report(active, it.span);
        }
        walk::walk_ts_this_type(self, it);
    }

    fn visit_ts_conditional_type(&mut self, it: &TSConditionalType<'a>) {
        if let Some(active) = self.active.no_conditional_types {
            self.report(active, it.span);
        }
        walk::walk_ts_conditional_type(self, it);
    }

    fn visit_ts_infer_type(&mut self, it: &TSInferType<'a>) {
        if let Some(active) = self.active.no_conditional_types {
            self.report(active, it.span);
        }
        walk::walk_ts_infer_type(self, it);
    }

    fn visit_ts_indexed_access_type(&mut self, it: &TSIndexedAccessType<'a>) {
        if let Some(active) = self.active.no_aliases_by_index {
            self.report(active, it.span);
        }
        walk::walk_ts_indexed_access_type(self, it);
    }

    fn visit_ts_type_query(&mut self, it: &TSTypeQuery<'a>) {
        if let Some(active) = self.active.no_type_query {
            self.report(active, it.span);
        }
        walk::walk_ts_type_query(self, it);
    }

    fn visit_ts_mapped_type(&mut self, it: &TSMappedType<'a>) {
        if let Some(active) = self.active.no_mapped_types {
            self.report(active, it.span);
        }
        walk::walk_ts_mapped_type(self, it);
    }

    fn visit_ts_constructor_type(&mut self, it: &TSConstructorType<'a>) {
        if let Some(active) = self.active.no_ctor_signatures_funcs {
            self.report(active, it.span);
        }
        walk::walk_ts_constructor_type(self, it);
    }

    fn visit_ts_type_predicate(&mut self, it: &TSTypePredicate<'a>) {
        if let Some(active) = self.active.no_is {
            self.report(active, it.span);
        }
        walk::walk_ts_type_predicate(self, it);
    }

    fn visit_ts_type_reference(&mut self, it: &TSTypeReference<'a>) {
        if let Some(active) = self.active.no_utility_types
            && let Some(name) = simple_type_name(&it.type_name)
            && is_unsupported_utility_type(name)
        {
            self.report(active, it.span);
        }
        walk::walk_ts_type_reference(self, it);
    }

    fn visit_ts_as_expression(&mut self, it: &TSAsExpression<'a>) {
        if let Some(active) = self.active.no_as_const
            && is_const_type_reference(&it.type_annotation)
        {
            self.report(active, it.type_annotation.span());
        }
        walk::walk_ts_as_expression(self, it);
    }

    fn visit_ts_import_equals_declaration(&mut self, it: &TSImportEqualsDeclaration<'a>) {
        if let Some(active) = self.active.no_require {
            self.report(active, it.span);
        }
        walk::walk_ts_import_equals_declaration(self, it);
    }

    fn visit_ts_export_assignment(&mut self, it: &TSExportAssignment<'a>) {
        if let Some(active) = self.active.no_export_assignment {
            self.report(active, it.span);
        }
        walk::walk_ts_export_assignment(self, it);
    }

    fn visit_ts_namespace_export_declaration(&mut self, it: &TSNamespaceExportDeclaration<'a>) {
        if let Some(active) = self.active.no_umd {
            self.report(active, it.span);
        }
        walk::walk_ts_namespace_export_declaration(self, it);
    }

    fn visit_ts_module_declaration(&mut self, it: &TSModuleDeclaration<'a>) {
        if it.declare
            && let Some(active) = self.active.no_ambient_decls
        {
            self.report(active, it.span);
        }
        if let TSModuleDeclarationName::StringLiteral(lit) = &it.id
            && lit.value.contains('*')
            && let Some(active) = self.active.no_module_wildcards
        {
            self.report(active, lit.span);
        }
        walk::walk_ts_module_declaration(self, it);
    }

    fn visit_spread_element(&mut self, it: &SpreadElement<'a>) {
        if let Some(active) = self.active.no_spread {
            self.report(active, it.span);
        }
        walk::walk_spread_element(self, it);
    }

    fn visit_object_property(&mut self, it: &ObjectProperty<'a>) {
        if let Some(active) = self.active.identifiers_as_prop_names
            && !it.computed
            && matches!(
                it.key,
                PropertyKey::StringLiteral(_) | PropertyKey::NumericLiteral(_)
            )
        {
            self.report(active, it.key.span());
        }
        walk::walk_object_property(self, it);
    }

    fn visit_computed_member_expression(&mut self, it: &ComputedMemberExpression<'a>) {
        if let Some(active) = self.active.no_props_by_index
            && !matches!(it.expression, Expression::NumericLiteral(_))
        {
            self.report(active, it.span);
        }
        walk::walk_computed_member_expression(self, it);
    }

    fn visit_static_member_expression(&mut self, it: &StaticMemberExpression<'a>) {
        if let Some(active) = self.active.system_api_version
            && let Some((api, span)) = self.system_api_key_from_static_member(it)
        {
            self.report_system_api_version(active, &api, span);
        }
        walk::walk_static_member_expression(self, it);
    }

    fn visit_sequence_expression(&mut self, it: &SequenceExpression<'a>) {
        if let Some(active) = self.active.no_comma_outside_loops {
            self.report(active, it.span);
        }
        walk::walk_sequence_expression(self, it);
    }

    fn visit_new_target(&mut self, it: &NewTarget) {
        if let Some(active) = self.active.no_new_target {
            self.report(active, it.span);
        }
        walk::walk_new_target(self, it);
    }
}

fn is_identifier(expression: &Expression<'_>, name: &str) -> bool {
    matches!(expression, Expression::Identifier(identifier) if identifier.name == name)
}

fn collect_system_api_imports<'a>(body: &[Statement<'a>]) -> HashMap<String, SystemImport> {
    let mut imports = HashMap::new();
    for statement in body {
        match statement {
            Statement::ImportDeclaration(declaration) => {
                collect_system_api_import_declaration(
                    declaration.source.value.as_str(),
                    declaration
                        .specifiers
                        .as_ref()
                        .map(|specifiers| specifiers.as_slice()),
                    &mut imports,
                );
            }
            Statement::LazyImportDeclaration(declaration) => {
                collect_system_api_import_declaration(
                    declaration.source.value.as_str(),
                    declaration
                        .specifiers
                        .as_ref()
                        .map(|specifiers| specifiers.as_slice()),
                    &mut imports,
                );
            }
            _ => {}
        }
    }
    imports
}

fn collect_system_api_import_declaration<'a>(
    source: &str,
    specifiers: Option<&[ImportDeclarationSpecifier<'a>]>,
    imports: &mut HashMap<String, SystemImport>,
) {
    let Some(module) = system_module_name(source) else {
        return;
    };

    let Some(specifiers) = specifiers else {
        return;
    };

    for specifier in specifiers {
        match specifier {
            ImportDeclarationSpecifier::ImportSpecifier(specifier) => {
                if let Some(imported) = module_export_name(&specifier.imported) {
                    imports.insert(
                        specifier.local.name.to_string(),
                        SystemImport {
                            module: module.to_string(),
                            imported: Some(imported.to_string()),
                        },
                    );
                }
            }
            ImportDeclarationSpecifier::ImportDefaultSpecifier(specifier) => {
                imports.insert(
                    specifier.local.name.to_string(),
                    SystemImport {
                        module: module.to_string(),
                        imported: None,
                    },
                );
            }
            ImportDeclarationSpecifier::ImportNamespaceSpecifier(specifier) => {
                imports.insert(
                    specifier.local.name.to_string(),
                    SystemImport {
                        module: module.to_string(),
                        imported: None,
                    },
                );
            }
        }
    }
}

fn module_export_name<'a>(name: &'a ModuleExportName<'a>) -> Option<&'a str> {
    match name {
        ModuleExportName::IdentifierName(identifier) => Some(identifier.name.as_str()),
        ModuleExportName::IdentifierReference(identifier) => Some(identifier.name.as_str()),
        ModuleExportName::StringLiteral(literal) => Some(literal.value.as_str()),
    }
}

fn system_module_name(source: &str) -> Option<&str> {
    if source.starts_with("@ohos.")
        || source.starts_with("@kit.")
        || source.starts_with("@system.")
        || source.starts_with("@hms.")
    {
        Some(source)
    } else {
        None
    }
}

fn system_api_version(options: &ArktsRuleOptions, api: &str) -> Option<SystemApiVersion> {
    options
        .system_api_versions
        .iter()
        .rev()
        .find_map(|(name, version)| (name == api).then_some(*version))
        .or_else(|| {
            SYSTEM_API_VERSIONS
                .iter()
                .find_map(|(name, version)| (*name == api).then_some(*version))
        })
}

fn static_member_property<'a>(expression: &'a Expression<'a>) -> Option<&'a str> {
    match expression {
        Expression::StaticMemberExpression(member) => Some(member.property.name.as_str()),
        _ => None,
    }
}

fn is_limited_stdlib_call(callee: &Expression<'_>, property_name: &str) -> bool {
    let Expression::StaticMemberExpression(member) = callee else {
        return false;
    };
    let Expression::Identifier(object) = &member.object else {
        return false;
    };

    match object.name.as_str() {
        "Object" => matches!(
            property_name,
            "__defineGetter__"
                | "__defineSetter__"
                | "__lookupGetter__"
                | "__lookupSetter__"
                | "assign"
                | "create"
                | "defineProperties"
                | "defineProperty"
                | "freeze"
                | "fromEntries"
                | "getOwnPropertyDescriptor"
                | "getOwnPropertyDescriptors"
                | "getOwnPropertySymbols"
                | "getPrototypeOf"
                | "hasOwnProperty"
                | "is"
                | "isExtensible"
                | "isFrozen"
                | "isPrototypeOf"
                | "isSealed"
                | "preventExtensions"
                | "propertyIsEnumerable"
                | "seal"
                | "setPrototypeOf"
        ),
        "Reflect" => matches!(
            property_name,
            "apply"
                | "construct"
                | "defineProperty"
                | "deleteProperty"
                | "getOwnPropertyDescriptor"
                | "getPrototypeOf"
                | "isExtensible"
                | "preventExtensions"
                | "setPrototypeOf"
        ),
        _ => false,
    }
}

fn is_prototype_assignment_target(target: &AssignmentTarget<'_>) -> bool {
    matches!(
        target,
        AssignmentTarget::StaticMemberExpression(member) if member.property.name == "prototype"
    )
}

fn is_new_error_expression(expression: &Expression<'_>) -> bool {
    match expression {
        Expression::NewExpression(new_expr) => matches!(
            &new_expr.callee,
            Expression::Identifier(identifier) if identifier.name == "Error"
        ),
        _ => false,
    }
}

fn is_const_type_reference(ty: &TSType<'_>) -> bool {
    matches!(ty, TSType::TSTypeReference(reference) if simple_type_name(&reference.type_name) == Some("const"))
}

fn simple_type_name<'a>(type_name: &'a TSTypeName<'a>) -> Option<&'a str> {
    match type_name {
        TSTypeName::IdentifierReference(identifier) => Some(identifier.name.as_str()),
        _ => None,
    }
}

fn is_unsupported_utility_type(name: &str) -> bool {
    matches!(
        name,
        "Pick"
            | "Omit"
            | "Exclude"
            | "Extract"
            | "NonNullable"
            | "Parameters"
            | "ConstructorParameters"
            | "ReturnType"
            | "InstanceType"
            | "ThisParameterType"
            | "OmitThisParameter"
            | "ThisType"
            | "Awaited"
            | "Uppercase"
            | "Lowercase"
            | "Capitalize"
            | "Uncapitalize"
    )
}

pub fn is_rule_name(name: &str) -> bool {
    find_rule(name).is_some()
}

pub(crate) fn rule_metas() -> impl Iterator<Item = ArktsRuleMeta> {
    ARKTS_RULES.iter().map(|rule| ArktsRuleMeta {
        name: rule.name,
        code: rule.code,
        message: rule.message,
        has_options: rule.check == ArktsCheck::SystemApiVersion,
    })
}

#[allow(dead_code)]
pub fn lint_standalone_file(
    file_path: &Path,
    rules: &[StandaloneRuleConfig],
    cwd: &Path,
) -> Result<Vec<StandaloneDiagnostic>, String> {
    let source_text = fs::read_to_string(file_path).map_err(|err| {
        format!(
            "Failed to read ArkTS source file `{}`: {err}",
            file_path.display()
        )
    })?;
    lint_standalone_source(file_path, &source_text, rules, cwd, None)
}

pub fn lint_standalone_source(
    file_path: &Path,
    source_text: &str,
    rules: &[StandaloneRuleConfig],
    cwd: &Path,
    source_type: Option<SourceType>,
) -> Result<Vec<StandaloneDiagnostic>, String> {
    if rules.is_empty() || !is_arkts_file(file_path) {
        return Ok(Vec::new());
    }

    let default_options = ArktsRuleOptions {
        min_api_version: find_project_min_api_version(cwd),
        system_api_versions: Vec::new(),
    };

    let mut active_rules = Vec::with_capacity(rules.len());
    let mut by_options_id = Vec::with_capacity(rules.len());
    for (index, config) in rules.iter().enumerate() {
        let rule = find_rule(&config.name)
            .ok_or_else(|| format!("Unknown ArkTS lint rule `arkts/{}`.", config.name))?;
        let parsed_options =
            parse_arkts_rule_options(rule, &config.options, cwd, &default_options)?;
        by_options_id.push(parsed_options);
        active_rules.push(ActiveArktsRule {
            active_index: index as u32,
            rule,
            options_id: index,
        });
    }

    let allocator = Allocator::default();
    let option_store = ArktsOptionStore {
        default: default_options,
        by_options_id,
    };

    run_arkts_rules(
        &file_path.to_string_lossy(),
        source_text,
        &allocator,
        &active_rules,
        option_store,
        source_type,
    )
    .map(|diagnostics| {
        diagnostics
            .into_iter()
            .filter_map(|diagnostic| {
                let config = rules.get(diagnostic.rule_index as usize)?;
                Some(StandaloneDiagnostic {
                    rule_name: config.name.clone(),
                    severity: config.severity,
                    message: diagnostic.message,
                    start: diagnostic.start,
                    end: diagnostic.end,
                })
            })
            .collect()
    })
}

fn find_rule(name: &str) -> Option<&'static ArktsRule> {
    ARKTS_RULES.iter().find(|rule| rule.name == name)
}
