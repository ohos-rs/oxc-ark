use std::{fs, io, path::Path};

use oxc_linter::Oxlintrc;
use schemars::{JsonSchema, r#gen::SchemaSettings};
use serde_json::{Map, Value, json};

use crate::arkts;

pub(crate) const CONFIGURATION_SCHEMA_PATH: &str =
    "./node_modules/@ohos-rs/oxk/configuration_schema.json";

pub fn configuration_schema_json() -> String {
    let mut schema = configuration_schema_value::<Oxlintrc>();
    patch_oxk_schema(&mut schema);
    inject_markdown_descriptions(&mut schema);
    serde_json::to_string_pretty(&schema).expect("configuration schema should serialize")
}

pub fn write_configuration_schema(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, format!("{}\n", configuration_schema_json()))
}

fn configuration_schema_value<T: JsonSchema>() -> Value {
    let generator = SchemaSettings::draft07()
        .with(|settings| {
            settings.option_add_null_type = false;
        })
        .into_generator();
    let mut schema = generator.into_root_schema_for::<T>();
    schema
        .schema
        .extensions
        .insert("allowComments".to_string(), Value::Bool(true));
    schema
        .schema
        .extensions
        .insert("allowTrailingCommas".to_string(), Value::Bool(true));
    serde_json::to_value(&schema).expect("configuration schema should convert to JSON")
}

fn patch_oxk_schema(schema: &mut Value) {
    rewrite_schema_references(schema);
    add_arkts_plugin(schema);
    add_arkts_rule_definitions(schema);
    add_arkts_rule_properties(schema);
}

fn rewrite_schema_references(value: &mut Value) {
    match value {
        Value::String(string) => {
            *string = string.replace(
                "./node_modules/oxlint/configuration_schema.json",
                CONFIGURATION_SCHEMA_PATH,
            );
        }
        Value::Object(map) => {
            for value in map.values_mut() {
                rewrite_schema_references(value);
            }
        }
        Value::Array(items) => {
            for item in items {
                rewrite_schema_references(item);
            }
        }
        _ => {}
    }
}

fn definitions_mut(schema: &mut Value) -> Option<&mut Map<String, Value>> {
    schema.get_mut("definitions")?.as_object_mut()
}

fn add_arkts_plugin(schema: &mut Value) {
    let Some(plugin_enum) = schema
        .pointer_mut("/definitions/LintPluginOptionsSchema/enum")
        .and_then(Value::as_array_mut)
    else {
        return;
    };

    if !plugin_enum
        .iter()
        .any(|value| value.as_str() == Some("arkts"))
    {
        plugin_enum.push(Value::String("arkts".to_string()));
    }
}

fn add_arkts_rule_definitions(schema: &mut Value) {
    let Some(definitions) = definitions_mut(schema) else {
        return;
    };

    definitions.insert(
        "ArktsRule".to_string(),
        json!({
            "description": "ArkTS lint rule configuration.",
            "anyOf": [
                { "$ref": "#/definitions/AllowWarnDeny" },
                {
                    "type": "array",
                    "items": [
                        { "$ref": "#/definitions/AllowWarnDeny" }
                    ],
                    "additionalItems": true,
                    "minItems": 1
                }
            ]
        }),
    );
    definitions.insert(
        "ArktsSystemApiVersionRule".to_string(),
        json!({
            "description": "ArkTS system API version lint rule configuration.",
            "anyOf": [
                { "$ref": "#/definitions/AllowWarnDeny" },
                {
                    "type": "array",
                    "items": [
                        { "$ref": "#/definitions/AllowWarnDeny" },
                        { "$ref": "#/definitions/ArktsSystemApiVersionOptions" }
                    ],
                    "additionalItems": false,
                    "minItems": 1,
                    "maxItems": 2
                }
            ]
        }),
    );
    definitions.insert(
        "ArktsSystemApiVersionOptions".to_string(),
        json!({
            "description": "Options for arkts/system-api-version.",
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "minApiVersion": version_value_schema("Minimum supported OpenHarmony API version."),
                "minVersion": version_value_schema("Alias of minApiVersion."),
                "apiVersionFile": {
                    "description": "Path to a JSON or JSONC file containing system API version data.",
                    "type": "string"
                },
                "apisFile": {
                    "description": "Alias of apiVersionFile.",
                    "type": "string"
                },
                "apis": {
                    "description": "Inline system API version data keyed by API name.",
                    "$ref": "#/definitions/ArktsSystemApiVersions"
                },
                "apiVersions": {
                    "description": "Alias of apis.",
                    "$ref": "#/definitions/ArktsSystemApiVersions"
                }
            }
        }),
    );
    definitions.insert(
        "ArktsSystemApiVersions".to_string(),
        json!({
            "type": "object",
            "additionalProperties": {
                "$ref": "#/definitions/ArktsSystemApiVersion"
            }
        }),
    );
    definitions.insert(
        "ArktsSystemApiVersion".to_string(),
        json!({
            "description": "System API version, either as a version number or an object with lifecycle fields.",
            "anyOf": [
                version_value_schema("The API version where this API was introduced."),
                {
                    "type": "object",
                    "additionalProperties": false,
                    "anyOf": [
                        { "required": ["since"] },
                        { "required": ["version"] },
                        { "required": ["apiVersion"] },
                        { "required": ["apiLevel"] }
                    ],
                    "properties": {
                        "since": version_value_schema("The API version where this API was introduced."),
                        "version": version_value_schema("Alias of since."),
                        "apiVersion": version_value_schema("Alias of since."),
                        "apiLevel": version_value_schema("Alias of since."),
                        "removed": version_value_schema("The API version where this API was removed."),
                        "removedVersion": version_value_schema("Alias of removed."),
                        "deleteVersion": version_value_schema("Alias of removed."),
                        "deletedVersion": version_value_schema("Alias of removed."),
                        "removalVersion": version_value_schema("Alias of removed."),
                        "deprecatedSince": version_value_schema("Alias of removed."),
                        "deprecatedVersion": version_value_schema("Alias of removed.")
                    }
                }
            ]
        }),
    );
}

fn add_arkts_rule_properties(schema: &mut Value) {
    let Some(rule_map) = schema
        .pointer_mut("/definitions/DummyRuleMap")
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    let properties = rule_map
        .entry("properties".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    let Some(properties) = properties.as_object_mut() else {
        return;
    };

    for rule in arkts::rule_metas() {
        let full_name = format!("arkts/{}", rule.name);
        let description = match rule.code {
            Some(code) => format!("{} ({code})", rule.message),
            None => rule.message.to_string(),
        };
        let ref_name = if rule.has_options {
            "#/definitions/ArktsSystemApiVersionRule"
        } else {
            "#/definitions/ArktsRule"
        };
        properties.insert(
            full_name,
            json!({
                "description": description,
                "allOf": [
                    { "$ref": ref_name }
                ]
            }),
        );
    }
}

fn version_value_schema(description: &str) -> Value {
    json!({
        "description": description,
        "anyOf": [
            {
                "type": "integer",
                "format": "uint32",
                "minimum": 0.0
            },
            {
                "type": "string",
                "pattern": "^[0-9]+$"
            }
        ]
    })
}

fn inject_markdown_descriptions(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(description)) = map.get("description") {
                map.insert(
                    "markdownDescription".to_string(),
                    Value::String(description.clone()),
                );
            }
            for value in map.values_mut() {
                inject_markdown_descriptions(value);
            }
        }
        Value::Array(items) => {
            for item in items {
                inject_markdown_descriptions(item);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use serde_json::Value;

    use super::configuration_schema_json;

    fn schema() -> Value {
        serde_json::from_str(&configuration_schema_json()).expect("schema should be valid JSON")
    }

    #[test]
    fn schema_includes_arkts_plugin() {
        let schema = schema();
        let plugins = schema
            .pointer("/definitions/LintPluginOptionsSchema/enum")
            .and_then(Value::as_array)
            .expect("plugin enum should exist");

        assert!(
            plugins
                .iter()
                .any(|plugin| plugin.as_str() == Some("arkts")),
            "schema should include the ArkTS plugin"
        );
    }

    #[test]
    fn schema_includes_arkts_rule_properties() {
        let schema = schema();
        let rules = schema
            .pointer("/definitions/DummyRuleMap/properties")
            .and_then(Value::as_object)
            .expect("rule properties should exist");

        assert!(rules.contains_key("arkts/no-symbol"));
        assert_eq!(
            rules
                .get("arkts/system-api-version")
                .and_then(|rule| rule.pointer("/allOf/0/$ref"))
                .and_then(Value::as_str),
            Some("#/definitions/ArktsSystemApiVersionRule")
        );
    }

    #[test]
    fn checked_in_schema_is_current() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../npm/oxk/configuration_schema.json");
        let checked_in = fs::read_to_string(&path).expect("checked-in schema should exist");
        let checked_in: Value =
            serde_json::from_str(&checked_in).expect("checked-in schema should be valid JSON");
        let generated: Value =
            serde_json::from_str(&configuration_schema_json()).expect("generated schema is JSON");

        assert_eq!(
            checked_in,
            generated,
            "run `pnpm run build:lint-schema` to update {}",
            path.display()
        );
    }
}
