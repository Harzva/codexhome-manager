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
    assert!(stdout.contains("registry"));
    assert!(stdout.contains("home"));
    assert!(stdout.contains("--json"));
    assert!(stdout.contains("Examples:"));
}

#[test]
fn lifecycle_subcommand_help_explains_safe_clone_controls() {
    let output = binary()
        .args(["home", "clone", "--help"])
        .output()
        .expect("run clone help");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    assert!(stdout.contains("--copy-capabilities"));
    assert!(stdout.contains("--dry-run"));
    assert!(stdout.contains("without copying authentication or sessions"));
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

#[test]
fn create_and_registry_list_form_a_json_lifecycle() {
    let temp = TempDir::new().expect("temp dir");
    let registry = temp.path().join("state/registry.json");
    let home = temp.path().join("frontend-home");

    let create = binary()
        .args(["home", "create", "@frontend", "--path"])
        .arg(&home)
        .args([
            "--label",
            "Frontend Family",
            "--specialty",
            "UI Design",
            "--specialty",
            "前端",
            "--registry",
        ])
        .arg(&registry)
        .arg("--json")
        .env("HOME", temp.path())
        .output()
        .expect("create Home");
    assert!(
        create.status.success(),
        "{}",
        String::from_utf8_lossy(&create.stderr)
    );
    let result: Value = serde_json::from_slice(&create.stdout).expect("create JSON");
    assert_eq!(result["schemaVersion"], "codexhome.home-mutation.v1");
    assert_eq!(result["entry"]["alias"], "@frontend");
    assert_eq!(result["entry"]["specialties"][0], "ui-design");
    assert!(home.join("config.toml").is_file());
    assert!(!home.join("auth.json").exists());

    let list = binary()
        .args(["registry", "list", "--registry"])
        .arg(&registry)
        .arg("--json")
        .env("HOME", temp.path())
        .output()
        .expect("list registry");
    assert!(list.status.success());
    let report: Value = serde_json::from_slice(&list.stdout).expect("registry JSON");
    assert_eq!(report["schemaVersion"], "codexhome.registry-report.v1");
    assert_eq!(report["revision"], 1);
    assert_eq!(report["homes"][0]["alias"], "@frontend");
    assert_eq!(report["homes"][0]["available"], true);
}

#[test]
fn dry_run_and_duplicate_alias_errors_are_automation_friendly() {
    let temp = TempDir::new().expect("temp dir");
    let registry = temp.path().join("registry.json");
    let first = temp.path().join("first");
    let planned = temp.path().join("planned");

    let dry_run = binary()
        .args(["home", "create", "@planned", "--path"])
        .arg(&planned)
        .args(["--dry-run", "--registry"])
        .arg(&registry)
        .arg("--json")
        .env("HOME", temp.path())
        .output()
        .expect("dry run");
    assert!(dry_run.status.success());
    let preview: Value = serde_json::from_slice(&dry_run.stdout).expect("preview JSON");
    assert_eq!(preview["dryRun"], true);
    assert!(!planned.exists());
    assert!(!registry.exists());

    let first_result = binary()
        .args(["home", "create", "@family", "--path"])
        .arg(&first)
        .args(["--registry"])
        .arg(&registry)
        .arg("--json")
        .env("HOME", temp.path())
        .output()
        .expect("first create");
    assert!(first_result.status.success());

    let duplicate = binary()
        .args(["home", "create", "@family", "--path"])
        .arg(temp.path().join("second"))
        .args(["--registry"])
        .arg(&registry)
        .arg("--json")
        .env("HOME", temp.path())
        .output()
        .expect("duplicate create");
    assert_eq!(duplicate.status.code(), Some(2));
    assert!(duplicate.stderr.is_empty());
    let error: Value = serde_json::from_slice(&duplicate.stdout).expect("error JSON");
    assert_eq!(error["ok"], false);
    assert!(error["error"]["message"]
        .as_str()
        .expect("message")
        .contains("already assigned"));
}
