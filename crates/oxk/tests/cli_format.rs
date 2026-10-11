use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "oxk-cli-format-{}-{nonce}-{id}",
            std::process::id()
        ));
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

#[test]
fn cargo_cli_formats_static_ets_with_explicit_language() {
    let temp = TempDir::new();
    let file_path = temp.path().join("input.ets");
    fs::write(
        &file_path,
        "package example.formatter;\nfinal class Box{value:int=1}\nlet character:char=c'a';\n",
    )
    .expect("failed to write fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_oxk"))
        .current_dir(temp.path())
        .args(["format", "--lang", "ets-static", "input.ets"])
        .output()
        .expect("failed to run static ETS format");

    assert!(
        output.status.success(),
        "static ETS format should succeed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let formatted = fs::read_to_string(file_path).expect("formatted fixture should be readable");
    assert!(formatted.contains("final class Box {"));
    assert!(formatted.contains("value: int = 1;"));
    assert!(formatted.contains("let character: char = c'a';"));
}

#[test]
fn cargo_cli_applies_editorconfig_sorting_and_embedded_options() {
    let temp = TempDir::new();
    fs::write(
        temp.path().join(".editorconfig"),
        "root = true\n[*]\nquote_type = single\n",
    )
    .unwrap();
    fs::write(
        temp.path().join("input.ts"),
        "import z from \"z\";import a from \"a\";const styles=css`a{color:red}`;",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_oxk"))
        .current_dir(temp.path())
        .args([
            "format",
            "--experimental-sort-imports",
            "{}",
            "--embedded-language-formatting",
            "off",
            "input.ts",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let code = fs::read_to_string(temp.path().join("input.ts")).unwrap();
    assert!(code.contains("import a from 'a';"), "{code}");
    assert!(code.find("import a") < code.find("import z"), "{code}");
    assert!(code.contains("a{color:red}"), "{code}");
}
