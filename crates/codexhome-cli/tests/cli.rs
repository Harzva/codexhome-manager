use serde_json::Value;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_codexhome"))
}

#[test]
fn root_help_is_useful() {
    let output = binary().arg("--help").output().expect("run help");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    assert!(stdout.contains("isolated Skill Spaces and Agent Households"));
    assert!(stdout.contains("scan"));
    assert!(stdout.contains("inspect"));
    assert!(stdout.contains("doctor"));
    assert!(stdout.contains("--json"));
}

#[test]
fn scan_json_is_clean_and_redacted() {
    let temp = TempDir::new().expect("temp dir");
    let home = temp.path().join(".codex");
    fs::create_dir_all(home.join("skills/research")).expect("skills");
    fs::write(home.join("auth.json"), r#"{"token":"must-not-escape"}"#).expect("auth");
    fs::write(home.join("config.toml"), "model = \"gpt-test\"").expect("config");

    let output = binary()
        .args(["scan", "--json"])
        .env("HOME", temp.path())
        .env_remove("CODEX_HOME")
        .env_remove("CODEXHOME_PATHS")
        .output()
        .expect("run scan");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    let report: Value = serde_json::from_str(&stdout).expect("JSON only");
    assert_eq!(report["schemaVersion"], "codexhome.discovery.v1");
    assert_eq!(report["homes"][0]["skillCount"], 1);
    assert_eq!(report["safety"]["readsAuthContents"], false);
    assert_eq!(report["safety"]["includesLocalPaths"], true);
    assert!(!stdout.contains("must-not-escape"));
}

#[test]
fn doctor_uses_exit_one_for_an_unhealthy_home() {
    let temp = TempDir::new().expect("temp dir");
    let home = temp.path().join(".codex");
    fs::create_dir_all(&home).expect("home");
    fs::write(home.join("config.toml"), "model = \"gpt-test\"").expect("config");

    let output = binary()
        .args(["doctor", "--json"])
        .env("HOME", temp.path())
        .env_remove("CODEX_HOME")
        .env_remove("CODEXHOME_PATHS")
        .output()
        .expect("run doctor");

    assert_eq!(output.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&output.stdout).expect("JSON output");
    assert_eq!(report["ok"], false);
    assert_eq!(report["homeCount"], 1);
    assert_eq!(report["healthyHomeCount"], 0);
}
