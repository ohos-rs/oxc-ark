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
