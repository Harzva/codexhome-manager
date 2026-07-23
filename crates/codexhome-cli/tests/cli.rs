use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_codexhome"))
}

fn run_json(store: &Path, args: &[&str]) -> Value {
    let output = binary()
        .args(args)
        .arg("--observability-store")
        .arg(store)
        .arg("--json")
        .output()
        .expect("run codexhome JSON command");
    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("valid JSON output")
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
    assert!(stdout.contains("observe"));
    assert!(stdout.contains("task"));
    assert!(stdout.contains("run"));
    assert!(stdout.contains("route"));
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

#[test]
fn observability_record_summary_verify_and_export_form_one_cli_flow() {
    let temp = TempDir::new().expect("temp dir");
    let input = temp.path().join("events.json");
    let store = temp.path().join("state/events.jsonl");
    let csv = temp.path().join("exports/events.csv");
    let safety = serde_json::json!({
        "secretsRedacted": true,
        "includesSecretValues": false,
        "readsAuthContents": false,
        "includesLocalPaths": false
    });
    let events = serde_json::json!([
        {
            "schemaVersion": "codexhome.observability-event.v1",
            "eventId": "evt-1",
            "timestampMs": 1,
            "eventType": "task_created",
            "status": "started",
            "trace": {"taskId": "task-1"},
            "identity": {},
            "safety": safety
        },
        {
            "schemaVersion": "codexhome.observability-event.v1",
            "eventId": "evt-2",
            "timestampMs": 2,
            "eventType": "run_started",
            "status": "started",
            "trace": {"taskId": "task-1", "runId": "run-1"},
            "identity": {},
            "safety": safety
        },
        {
            "schemaVersion": "codexhome.observability-event.v1",
            "eventId": "evt-3",
            "timestampMs": 3,
            "eventType": "attempt_started",
            "status": "running",
            "trace": {
                "taskId": "task-1",
                "runId": "run-1",
                "attemptId": "attempt-1"
            },
            "identity": {
                "homeId": "home-1",
                "homeAlias": "@research",
                "accountId": "account-1",
                "model": "gpt-test"
            },
            "safety": safety
        },
        {
            "schemaVersion": "codexhome.observability-event.v1",
            "eventId": "evt-4",
            "timestampMs": 4,
            "eventType": "thread_linked",
            "status": "running",
            "trace": {
                "taskId": "task-1",
                "runId": "run-1",
                "threadId": "thread-1"
            },
            "identity": {},
            "safety": safety
        },
        {
            "schemaVersion": "codexhome.observability-event.v1",
            "eventId": "evt-5",
            "timestampMs": 5,
            "eventType": "attempt_completed",
            "status": "succeeded",
            "trace": {
                "taskId": "task-1",
                "runId": "run-1",
                "attemptId": "attempt-1",
                "threadId": "thread-1"
            },
            "identity": {
                "homeId": "home-1",
                "homeAlias": "@research",
                "accountId": "account-1",
                "model": "gpt-test"
            },
            "usage": {
                "inputTokens": 100,
                "outputTokens": 25,
                "cachedInputTokens": 80,
                "cacheHits": 4,
                "cacheMisses": 1,
                "durationMs": 750,
                "estimatedCostMicrousd": 900,
                "retries": 1
            },
            "safety": safety
        }
    ]);
    fs::write(
        &input,
        serde_json::to_vec_pretty(&events).expect("serialize events"),
    )
    .expect("write events");

    let record = binary()
        .args(["observe", "record"])
        .arg(&input)
        .arg("--observability-store")
        .arg(&store)
        .arg("--json")
        .output()
        .expect("record events");
    assert!(
        record.status.success(),
        "{}",
        String::from_utf8_lossy(&record.stderr)
    );
    let report: Value = serde_json::from_slice(&record.stdout).expect("record JSON");
    assert_eq!(report["appended"], 5);
    assert_eq!(report["totalEvents"], 5);

    let summary = binary()
        .args(["observe", "summary", "--home-id", "@research"])
        .arg("--observability-store")
        .arg(&store)
        .arg("--json")
        .output()
        .expect("summarize events");
    assert!(
        summary.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&summary.stdout),
        String::from_utf8_lossy(&summary.stderr)
    );
    let report: Value = serde_json::from_slice(&summary.stdout).expect("summary JSON");
    assert_eq!(
        report["schemaVersion"],
        "codexhome.observability-summary.v1"
    );
    assert_eq!(report["totals"]["runs"], 1);
    assert_eq!(report["totals"]["attempts"], 1);
    assert_eq!(report["totals"]["totalTokens"], 125);
    assert_eq!(report["totals"]["cacheHitRateBasisPoints"], 8000);
    assert_eq!(report["totals"]["durationMs"], 750);

    let verify = binary()
        .args(["observe", "verify"])
        .arg("--observability-store")
        .arg(&store)
        .arg("--json")
        .output()
        .expect("verify events");
    assert!(verify.status.success());
    let report: Value = serde_json::from_slice(&verify.stdout).expect("verify JSON");
    assert_eq!(report["eventCount"], 5);
    assert_eq!(report["threadCount"], 1);

    let export = binary()
        .args(["observe", "export", "--format", "csv", "--output"])
        .arg(&csv)
        .arg("--observability-store")
        .arg(&store)
        .arg("--json")
        .output()
        .expect("export events");
    assert!(export.status.success());
    let csv_contents = fs::read_to_string(csv).expect("read CSV");
    assert!(csv_contents.starts_with("schema_version,event_id"));
    assert!(csv_contents.contains("attempt_completed"));
    assert!(csv_contents.contains("gpt-test"));
}

#[test]
fn agent_run_cli_tracks_migration_failure_cost_and_final_artifact() {
    let temp = TempDir::new().expect("temp dir");
    let store = temp.path().join("state/events.jsonl");

    let task = run_json(
        &store,
        &[
            "task",
            "create",
            "--task-id",
            "task-cli",
            "--label",
            "CLI lifecycle",
            "--kind",
            "coding",
        ],
    );
    assert_eq!(task["schemaVersion"], "codexhome.agent-run-mutation.v1");
    assert_eq!(task["taskId"], "task-cli");

    run_json(
        &store,
        &[
            "run",
            "start",
            "task-cli",
            "--run-id",
            "run-cli",
            "--max-total-tokens",
            "2000",
            "--max-attempts",
            "3",
        ],
    );
    run_json(
        &store,
        &[
            "run",
            "attempt",
            "start",
            "run-cli",
            "--attempt-id",
            "attempt-1",
            "--home-id",
            "home-a",
            "--model",
            "gpt-a",
            "--route-reason",
            "initial coding route",
        ],
    );
    run_json(
        &store,
        &[
            "run",
            "attempt",
            "fail",
            "run-cli",
            "attempt-1",
            "--input-tokens",
            "500",
            "--output-tokens",
            "100",
            "--duration-ms",
            "900",
            "--estimated-cost-microusd",
            "300",
            "--failure-code",
            "rate_limited",
            "--failure-reason",
            "provider limit",
            "--retryable",
        ],
    );
    run_json(
        &store,
        &[
            "run",
            "attempt",
            "migrate",
            "run-cli",
            "attempt-1",
            "--attempt-id",
            "attempt-2",
            "--home-id",
            "home-b",
            "--model",
            "gpt-b",
            "--route-reason",
            "move away from the limited account",
        ],
    );
    run_json(
        &store,
        &[
            "run",
            "thread",
            "link",
            "run-cli",
            "thread-cli",
            "--attempt-id",
            "attempt-2",
        ],
    );
    run_json(
        &store,
        &[
            "run",
            "artifact",
            "record",
            "run-cli",
            "attempt-2",
            "--thread-id",
            "thread-cli",
            "--artifact-id",
            "artifact-cli",
            "--artifact-kind",
            "patch",
        ],
    );
    run_json(
        &store,
        &[
            "run",
            "verification",
            "record",
            "run-cli",
            "attempt-2",
            "--thread-id",
            "thread-cli",
            "--verification-id",
            "verification-cli",
            "--verification-kind",
            "tests",
            "--target-artifact-id",
            "artifact-cli",
            "--duration-ms",
            "250",
        ],
    );
    run_json(
        &store,
        &[
            "run",
            "attempt",
            "complete",
            "run-cli",
            "attempt-2",
            "--thread-id",
            "thread-cli",
            "--input-tokens",
            "700",
            "--output-tokens",
            "200",
            "--duration-ms",
            "1200",
            "--estimated-cost-microusd",
            "450",
        ],
    );
    run_json(
        &store,
        &[
            "run",
            "complete",
            "run-cli",
            "--final-artifact-id",
            "artifact-cli",
        ],
    );

    let thread_summary = run_json(&store, &["observe", "summary", "--thread", "thread-cli"]);
    assert_eq!(thread_summary["totals"]["totalTokens"], 900);
    assert_eq!(thread_summary["totals"]["durationMs"], 1200);

    let report = run_json(&store, &["run", "show", "run-cli"]);
    assert_eq!(report["schemaVersion"], "codexhome.agent-run.v1");
    assert_eq!(report["run"]["status"], "succeeded");
    assert_eq!(
        report["run"]["attempts"]
            .as_array()
            .expect("attempts")
            .len(),
        2
    );
    assert_eq!(report["run"]["consumption"]["totalTokens"], 1500);
    assert_eq!(report["run"]["consumption"]["failedAttemptTokens"], 600);
    assert_eq!(
        report["run"]["consumption"]["failedAttemptCostMicrousd"],
        300
    );
    assert_eq!(report["run"]["finalArtifactIds"][0], "artifact-cli");
    assert_eq!(report["run"]["verifications"][0]["succeeded"], true);
    assert_eq!(report["run"]["threadIds"][0], "thread-cli");
    assert_eq!(report["task"]["status"], "succeeded");
}

#[test]
fn route_cli_records_explainable_decision_and_binds_attempt_identity() {
    let temp = TempDir::new().expect("temp dir");
    let registry = temp.path().join("state/registry.json");
    let store = temp.path().join("state/events.jsonl");
    let policy_path = temp.path().join("router-policy.json");
    let request_path = temp.path().join("route-request.json");

    let create_home = |alias: &str, path: &Path, specialty: &str| {
        let output = binary()
            .args(["home", "create", alias, "--path"])
            .arg(path)
            .args(["--specialty", specialty, "--registry"])
            .arg(&registry)
            .arg("--json")
            .env("HOME", temp.path())
            .output()
            .expect("create route Home");
        assert!(
            output.status.success(),
            "stdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<Value>(&output.stdout).expect("Home JSON")
    };
    let spark = create_home("@spark", &temp.path().join("spark-home"), "spark");
    let primary = create_home(
        "@primary",
        &temp.path().join("primary-home"),
        "architecture",
    );
    let policy = serde_json::json!({
        "schemaVersion": "codexhome.route-policy.v1",
        "policyId": "cli-policy",
        "revision": 1,
        "taskRules": [{
            "taskKind": "simple_code",
            "preferredSpecialties": ["spark"],
            "requiredCapabilities": ["coding"],
            "minimumModelStrengthBasisPoints": null,
            "weights": null
        }],
        "candidates": [
            {
                "candidateId": "spark",
                "homeId": spark["entry"]["id"],
                "homeAlias": "@spark",
                "accountId": "spark-account",
                "provider": "test",
                "model": "spark-model",
                "specialties": ["spark", "coding"],
                "capabilities": ["coding"],
                "securityDomains": ["local"],
                "maxContextTokens": 128000,
                "modelStrengthBasisPoints": 6500,
                "inputCostMicrousdPerMillionTokens": 100,
                "outputCostMicrousdPerMillionTokens": 200,
                "maxConcurrentRuns": 4,
                "baselineDurationMs": 10000
            },
            {
                "candidateId": "primary",
                "homeId": primary["entry"]["id"],
                "homeAlias": "@primary",
                "accountId": "primary-account",
                "provider": "test",
                "model": "strong-model",
                "specialties": ["architecture", "coding"],
                "capabilities": ["coding"],
                "securityDomains": ["local", "local-sensitive"],
                "maxContextTokens": 512000,
                "modelStrengthBasisPoints": 9500,
                "inputCostMicrousdPerMillionTokens": 900,
                "outputCostMicrousdPerMillionTokens": 1800,
                "maxConcurrentRuns": 2,
                "baselineDurationMs": 40000
            }
        ]
    });
    fs::write(
        &policy_path,
        serde_json::to_string_pretty(&policy).expect("policy JSON"),
    )
    .expect("write policy");
    fs::write(
        &request_path,
        serde_json::to_string_pretty(&serde_json::json!({
            "schemaVersion": "codexhome.route-request.v1",
            "requestId": "request-cli",
            "taskKind": "simple_code",
            "estimatedContextTokens": 12000,
            "estimatedOutputTokens": 2000,
            "requiredCapabilities": ["coding"],
            "preferredSpecialties": ["spark"],
            "sensitiveDirectory": false,
            "requiredSecurityDomain": null,
            "lockedHome": null,
            "lockedModel": null,
            "maxEstimatedCostMicrousd": null,
            "allowDegraded": false
        }))
        .expect("request JSON"),
    )
    .expect("write request");

    run_json(
        &store,
        &[
            "task",
            "create",
            "--task-id",
            "task-route",
            "--label",
            "Route CLI task",
            "--kind",
            "simple_code",
        ],
    );
    run_json(
        &store,
        &["run", "start", "task-route", "--run-id", "run-route"],
    );
    let decision_output = binary()
        .args(["route", "decide"])
        .arg(&request_path)
        .args(["--run-id", "run-route", "--route-policy"])
        .arg(&policy_path)
        .arg("--registry")
        .arg(&registry)
        .arg("--observability-store")
        .arg(&store)
        .arg("--json")
        .env("HOME", temp.path())
        .output()
        .expect("route decide");
    assert!(
        decision_output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&decision_output.stdout),
        String::from_utf8_lossy(&decision_output.stderr)
    );
    let decision: Value =
        serde_json::from_slice(&decision_output.stdout).expect("route decision JSON");
    assert_eq!(
        decision["schemaVersion"],
        "codexhome.route-decision-record.v1"
    );
    assert_eq!(decision["decision"]["selected"]["candidateId"], "spark");
    assert_eq!(
        decision["decision"]["candidates"][0]["components"]
            .as_array()
            .map(Vec::len),
        Some(9)
    );
    assert_eq!(decision["decision"]["observedEventCount"], 2);
    assert_eq!(
        decision["decision"]["request"]["estimatedContextTokens"],
        12_000
    );
    let route_event = decision["eventId"].as_str().expect("route event ID");

    let attempt = binary()
        .args([
            "run",
            "attempt",
            "start",
            "run-route",
            "--attempt-id",
            "attempt-route",
            "--home-id",
        ])
        .arg(spark["entry"]["id"].as_str().expect("spark id"))
        .args([
            "--home-alias",
            "@spark",
            "--account",
            "spark-account",
            "--provider",
            "test",
            "--model",
            "spark-model",
            "--route-reason",
            "follow explainable route decision",
            "--route-decision-id",
            route_event,
            "--observability-store",
        ])
        .arg(&store)
        .arg("--json")
        .output()
        .expect("start routed attempt");
    assert!(
        attempt.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&attempt.stdout),
        String::from_utf8_lossy(&attempt.stderr)
    );
    let report = run_json(&store, &["run", "show", "run-route"]);
    assert_eq!(report["run"]["routeDecisions"][0]["eventId"], route_event);
    assert_eq!(report["run"]["routeDecisions"][0]["observedEventCount"], 2);
    assert_eq!(
        report["run"]["routeDecisions"][0]["evaluatedAtTimestampMs"],
        report["run"]["routeDecisions"][0]["decidedAtMs"]
    );
    assert_eq!(
        report["run"]["routeDecisions"][0]["request"]["taskKind"],
        "simple_code"
    );
    assert_eq!(
        report["run"]["routeDecisions"][0]["candidates"][0]["activeAttempts"],
        0
    );
    assert!(
        report["run"]["routeDecisions"][0]["candidates"][0]["components"][0]["reason"]
            .as_str()
            .is_some_and(|reason| !reason.is_empty())
    );
    assert_eq!(report["run"]["attempts"][0]["routeDecisionId"], route_event);
}
