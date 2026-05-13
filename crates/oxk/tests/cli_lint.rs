use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::Value;

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("oxk-cli-lint-{}-{nonce}-{id}", std::process::id()));
        fs::create_dir_all(&path).expect("failed to create temp dir");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn lint_json(output: &std::process::Output) -> Value {
    let stdout = String::from_utf8(output.stdout.clone()).expect("stdout should be utf-8");
    let stderr = String::from_utf8(output.stderr.clone()).expect("stderr should be utf-8");
    serde_json::from_str(format!("{stdout}\n{stderr}").trim()).unwrap_or_else(|err| {
        panic!("lint output should be json: {err}\nstdout:\n{stdout}\nstderr:\n{stderr}")
    })
}

fn diagnostic_codes(report: &Value) -> Vec<String> {
    report["diagnostics"]
        .as_array()
        .expect("diagnostics should be an array")
        .iter()
        .filter_map(|diagnostic| diagnostic["code"].as_str().map(str::to_owned))
        .collect()
}

#[test]
fn cargo_cli_lint_reports_json_diagnostic() {
    let temp = TempDir::new();
    let file_path = temp.path().join("input.ts");
    fs::write(&file_path, "debugger\n").expect("failed to write fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_oxk"))
        .current_dir(temp.path())
        .args([
            "lint",
            "input.ts",
            "--threads",
            "1",
            "--format",
            "json",
            "-D",
            "no-debugger",
        ])
        .output()
        .expect("failed to run oxk lint");

    assert!(!output.status.success(), "lint should fail on no-debugger");

    let report = lint_json(&output);

    assert_eq!(report["number_of_files"], 1);
    assert_eq!(report["diagnostics"][0]["code"], "eslint(no-debugger)");
    assert_eq!(
        report["diagnostics"][0]["filename"].as_str(),
        Some("input.ts")
    );
}

#[test]
fn cargo_cli_lint_arkts_plugin_does_not_enable_rules_by_default() {
    let temp = TempDir::new();
    fs::write(
        temp.path().join(".oxlintrc.json"),
        serde_json::json!({
            "plugins": ["arkts"],
            "rules": {
                "no-unused-vars": "off"
            }
        })
        .to_string(),
    )
    .expect("failed to write config");
    fs::write(temp.path().join("input.ets"), "const key = Symbol('id')\n")
        .expect("failed to write fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_oxk"))
        .current_dir(temp.path())
        .args(["lint", "input.ets", "--threads", "1"])
        .output()
        .expect("failed to run oxk lint");

    assert!(
        output.status.success(),
        "arkts plugin registration alone should not enable rules: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn cargo_cli_lint_arkts_rules_only_run_for_ets() {
    let temp = TempDir::new();
    fs::write(
        temp.path().join(".oxlintrc.json"),
        serde_json::json!({
            "plugins": ["arkts"],
            "rules": {
                "no-unused-vars": "off",
                "arkts/no-symbol": "error"
            }
        })
        .to_string(),
    )
    .expect("failed to write config");
    fs::write(temp.path().join("input.ts"), "const key = Symbol('id')\n")
        .expect("failed to write fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_oxk"))
        .current_dir(temp.path())
        .args(["lint", "input.ts", "--threads", "1"])
        .output()
        .expect("failed to run oxk lint");

    assert!(
        output.status.success(),
        "arkts rules should only run for ETS files: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn cargo_cli_lint_arkts_no_symbol_reports_symbol_constructs() {
    let temp = TempDir::new();
    fs::write(
        temp.path().join(".oxlintrc.jsonc"),
        r#"{
          // ArkTS rules are opt-in even after the plugin is registered.
          "plugins": ["arkts"],
          "rules": {
            "no-unused-vars": "off",
            "arkts/no-symbol": "error"
          }
        }"#,
    )
    .expect("failed to write config");
    fs::write(
        temp.path().join("input.ets"),
        "const iterator = Symbol.iterator\nconst key = Symbol('id')\nlet marker: symbol\n",
    )
    .expect("failed to write fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_oxk"))
        .current_dir(temp.path())
        .args(["lint", "input.ets", "--threads", "1", "--format", "json"])
        .output()
        .expect("failed to run oxk lint");

    assert!(
        !output.status.success(),
        "lint should fail on arkts/no-symbol"
    );
    let report = lint_json(&output);
    let codes = diagnostic_codes(&report);
    assert_eq!(codes, vec!["arkts(no-symbol)", "arkts(no-symbol)"]);
}

#[test]
fn cargo_cli_lint_arkts_reports_ast_stable_rules() {
    let temp = TempDir::new();
    fs::write(
        temp.path().join(".oxlintrc.json"),
        serde_json::json!({
            "plugins": ["arkts"],
            "rules": {
                "no-unused-vars": "off",
                "arkts/no-var": "error",
                "arkts/no-any-unknown": "error",
                "arkts/no-private-identifiers": "error",
                "arkts/no-definite-assignment": "error"
            }
        })
        .to_string(),
    )
    .expect("failed to write config");
    fs::write(
        temp.path().join("input.ets"),
        "var value: any\nclass Example {\n  #secret = 1\n  field!: string\n}\n",
    )
    .expect("failed to write fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_oxk"))
        .current_dir(temp.path())
        .args(["lint", "input.ets", "--threads", "1", "--format", "json"])
        .output()
        .expect("failed to run oxk lint");

    assert!(
        !output.status.success(),
        "lint should fail on configured ArkTS rules"
    );
    let report = lint_json(&output);
    let codes = diagnostic_codes(&report);
    assert!(codes.contains(&"arkts(no-var)".to_string()));
    assert!(codes.contains(&"arkts(no-any-unknown)".to_string()));
    assert!(codes.contains(&"arkts(no-private-identifiers)".to_string()));
    assert!(codes.contains(&"arkts(no-definite-assignment)".to_string()));
}
