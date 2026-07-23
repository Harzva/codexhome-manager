use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use codexhome_core::{
    discover, doctor, generate_id, observability_events_csv, parse_observability_events,
    AgentRunAction, AgentRunMutationReport, AgentRunReport, AgentRunStore, AgentRunView,
    AttemptTransition, AttemptTransitionKind, CodexHomeSummary, DiscoveryOptions, DiscoveryReport,
    ExecutionIdentity, FailureInfo, HomeManager, HomeMutationResult, ObservabilityFilter,
    ObservabilityStatus, ObservabilityStore, ObservabilitySummary, RegisteredHomeView,
    RegistryReport, RegistryStore, RunBudget, UsageDelta,
};
use serde_json::json;
use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Debug, Parser)]
#[command(
    name = "codexhome",
    version,
    about = "Discover and manage specialized Codex Homes",
    long_about = "Discover, register, and manage multiple CODEX_HOME directories as isolated Skill Spaces and Agent Households.\n\nCredential contents are never included in command output.",
    after_help = "Examples:\n  codexhome scan\n  codexhome registry list\n  codexhome task create --label 'Compile benchmark'\n  codexhome run start task-123 --max-total-tokens 300000\n  codexhome run attempt start run-123 --home-id home-main --model gpt-5.5\n  codexhome observe summary --home-id @research --json"
)]
struct Cli {
    /// Add a CODEX_HOME candidate without changing the active shell environment.
    #[arg(long, global = true, value_name = "PATH")]
    home: Vec<PathBuf>,

    /// Override the registry file (or set CODEXHOME_REGISTRY).
    #[arg(long, global = true, value_name = "FILE")]
    registry: Option<PathBuf>,

    /// Override the append-only observability JSONL store.
    #[arg(long, global = true, value_name = "FILE")]
    observability_store: Option<PathBuf>,

    /// Emit stable machine-readable JSON to stdout.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Discover local Codex Homes and summarize their capabilities.
    Scan,

    /// Inspect one discovered Home by ID, label, provider, or absolute path.
    Inspect {
        /// ID, label, provider, or path returned by `codexhome scan`.
        target: String,
    },

    /// Check discovered Homes for common configuration problems.
    Doctor,

    /// Read the persistent Home registry used by aliases and the future Desktop UI.
    Registry {
        #[command(subcommand)]
        command: RegistryCommand,
    },

    /// Create, import, or safely clone registered Homes.
    Home {
        #[command(subcommand)]
        command: HomeCommand,
    },

    /// Record, verify, summarize, and export task/run observability events.
    Observe {
        #[command(subcommand)]
        command: ObserveCommand,
    },

    /// Create and inspect durable task identities.
    Task {
        #[command(subcommand)]
        command: TaskCommand,
    },

    /// Operate durable Agent Runs, attempts, threads, artifacts, and verification.
    Run {
        #[command(subcommand)]
        command: RunCommand,
    },
}

#[derive(Debug, Subcommand)]
enum TaskCommand {
    /// Create a task without storing its prompt or private payload.
    Create {
        #[arg(long)]
        task_id: Option<String>,
        #[arg(long)]
        label: String,
        #[arg(long)]
        kind: Option<String>,
    },
    /// List task projections from the append-only event stream.
    List,
    /// Show one task and all of its runs.
    Show { task_id: String },
}

#[derive(Debug, Subcommand)]
enum RunCommand {
    /// Start a run for an existing task.
    Start {
        task_id: String,
        #[arg(long)]
        run_id: Option<String>,
        #[command(flatten)]
        budget: BudgetArgs,
    },
    /// List run projections, optionally for one task.
    List {
        #[arg(long)]
        task_id: Option<String>,
    },
    /// Show one run with attempts, costs, artifacts, and recovery state.
    Show { run_id: String },
    /// Start, retry, migrate, complete, or fail an Agent attempt.
    Attempt {
        #[command(subcommand)]
        command: AttemptCommand,
    },
    /// Link a Codex thread or fork to a run and optional attempt.
    Thread {
        #[command(subcommand)]
        command: ThreadCommand,
    },
    /// Record a completed tool call.
    Tool {
        #[command(subcommand)]
        command: ToolCommand,
    },
    /// Register an opaque artifact identity and kind.
    Artifact {
        #[command(subcommand)]
        command: ArtifactCommand,
    },
    /// Record verification evidence for an attempt or artifact.
    Verification {
        #[command(subcommand)]
        command: VerificationCommand,
    },
    /// Complete a run after all attempts are terminal.
    Complete {
        run_id: String,
        #[arg(long = "final-artifact-id")]
        final_artifact_ids: Vec<String>,
    },
    /// Fail, cancel, or time out a run after all attempts are terminal.
    Fail {
        run_id: String,
        #[arg(long, value_enum, default_value_t = FailureStatusArg::Failed)]
        status: FailureStatusArg,
        #[command(flatten)]
        failure: FailureArgs,
    },
    /// Print the shared Agent Run and observability event-store path.
    Path,
}

#[derive(Debug, Subcommand)]
enum AttemptCommand {
    /// Start an initial attempt on one Home and model.
    Start {
        run_id: String,
        #[arg(long)]
        attempt_id: Option<String>,
        #[command(flatten)]
        identity: IdentityArgs,
        #[arg(long)]
        route_reason: Option<String>,
    },
    /// Retry a retryable failed attempt in the same run.
    Retry {
        run_id: String,
        from_attempt_id: String,
        #[arg(long)]
        attempt_id: Option<String>,
        #[command(flatten)]
        identity: IdentityArgs,
        #[arg(long)]
        route_reason: String,
    },
    /// Migrate a failed attempt to a different Home, account, or model.
    Migrate {
        run_id: String,
        from_attempt_id: String,
        #[arg(long)]
        attempt_id: Option<String>,
        #[command(flatten)]
        identity: IdentityArgs,
        #[arg(long)]
        route_reason: String,
    },
    /// Mark an attempt successful and record its total usage.
    Complete {
        run_id: String,
        attempt_id: String,
        #[arg(long)]
        thread_id: Option<String>,
        #[command(flatten)]
        usage: TerminalUsageArgs,
    },
    /// Mark an attempt failed, cancelled, or timed out.
    Fail {
        run_id: String,
        attempt_id: String,
        #[arg(long)]
        thread_id: Option<String>,
        #[arg(long, value_enum, default_value_t = FailureStatusArg::Failed)]
        status: FailureStatusArg,
        #[command(flatten)]
        usage: TerminalUsageArgs,
        #[command(flatten)]
        failure: FailureArgs,
    },
}

#[derive(Debug, Subcommand)]
enum ThreadCommand {
    Link {
        run_id: String,
        thread_id: String,
        #[arg(long)]
        attempt_id: Option<String>,
        #[arg(long)]
        parent_thread_id: Option<String>,
        #[arg(long)]
        fork_id: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
enum ToolCommand {
    Record {
        run_id: String,
        attempt_id: String,
        #[arg(long)]
        thread_id: Option<String>,
        #[arg(long)]
        tool_name: String,
        #[command(flatten)]
        usage: ToolUsageArgs,
    },
}

#[derive(Debug, Subcommand)]
enum ArtifactCommand {
    Record {
        run_id: String,
        attempt_id: String,
        #[arg(long)]
        thread_id: Option<String>,
        #[arg(long)]
        artifact_id: Option<String>,
        #[arg(long)]
        artifact_kind: String,
    },
}

#[derive(Debug, Subcommand)]
enum VerificationCommand {
    Record {
        run_id: String,
        attempt_id: String,
        #[arg(long)]
        thread_id: Option<String>,
        #[arg(long)]
        verification_id: Option<String>,
        #[arg(long)]
        verification_kind: String,
        #[arg(long)]
        target_artifact_id: Option<String>,
        #[arg(long, default_value_t = true)]
        succeeded: bool,
        #[arg(long, default_value_t = 0)]
        duration_ms: u64,
    },
}

#[derive(Debug, Subcommand)]
enum RegistryCommand {
    /// List registered Homes with live availability and capability summaries.
    List,

    /// Show one registered Home by @alias, ID, label, or path.
    Show {
        /// Registered @alias, ID, label, or absolute path.
        target: String,
    },

    /// Print the active registry file path.
    Path,
}

#[derive(Debug, Subcommand)]
enum HomeCommand {
    /// Create and register a new empty Home.
    Create {
        /// Unique family alias, for example @frontend.
        alias: String,

        /// Absolute path for the new CODEX_HOME.
        #[arg(long, value_name = "PATH")]
        path: PathBuf,

        /// Human-readable family name.
        #[arg(long, value_name = "NAME")]
        label: Option<String>,

        /// Specialty tag; repeat for more than one.
        #[arg(long, value_name = "TAG")]
        specialty: Vec<String>,

        /// Validate and print the plan without changing files or the registry.
        #[arg(long)]
        dry_run: bool,
    },

    /// Register an existing Home without modifying it.
    Import {
        /// Existing CODEX_HOME directory.
        path: PathBuf,

        /// Unique family alias, for example @research.
        #[arg(long)]
        alias: String,

        /// Human-readable family name.
        #[arg(long, value_name = "NAME")]
        label: Option<String>,

        /// Specialty tag; repeat for more than one.
        #[arg(long, value_name = "TAG")]
        specialty: Vec<String>,

        /// Validate and print the plan without changing the registry.
        #[arg(long)]
        dry_run: bool,
    },

    /// Safely clone a registered Home without copying authentication or sessions.
    Clone {
        /// Source @alias, ID, label, or registered path.
        source: String,

        /// Unique alias for the cloned family.
        alias: String,

        /// Absolute path for the cloned CODEX_HOME.
        #[arg(long, value_name = "PATH")]
        path: PathBuf,

        /// Human-readable family name.
        #[arg(long, value_name = "NAME")]
        label: Option<String>,

        /// Specialty tag; repeat for more than one.
        #[arg(long, value_name = "TAG")]
        specialty: Vec<String>,

        /// Copy Skills, Rules, and Hooks after secret-name and symlink filtering.
        #[arg(long)]
        copy_capabilities: bool,

        /// Validate and inspect the copy plan without changing anything.
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Debug, Subcommand)]
enum ObserveCommand {
    /// Append one event, a JSON array, or JSONL events to the store.
    Record {
        /// JSON/JSONL input file, or - for stdin.
        #[arg(value_name = "FILE")]
        input: String,
    },

    /// Aggregate token, duration, cache, retry, failure, and health metrics.
    Summary {
        #[command(flatten)]
        filter: ObservabilityFilterArgs,
    },

    /// Export filtered events as versioned JSON or flat CSV.
    Export {
        #[command(flatten)]
        filter: ObservabilityFilterArgs,

        #[arg(long, value_enum, default_value_t = ExportFormat::Json)]
        format: ExportFormat,

        #[arg(long, value_name = "FILE")]
        output: PathBuf,
    },

    /// Validate schemas, ordering, IDs, and task/run/attempt/thread linkage.
    Verify,

    /// Print the active observability store path.
    Path,
}

#[derive(Debug, Args, Default)]
struct ObservabilityFilterArgs {
    /// Match a Home ID or @alias.
    #[arg(long = "home-id", alias = "home-alias")]
    home_id: Option<String>,

    /// Match an account ID.
    #[arg(long)]
    account: Option<String>,

    /// Match a model name.
    #[arg(long)]
    model: Option<String>,

    /// Match a linked Codex thread ID.
    #[arg(long)]
    thread: Option<String>,

    /// Match a run ID.
    #[arg(long)]
    run: Option<String>,
}

impl From<ObservabilityFilterArgs> for ObservabilityFilter {
    fn from(value: ObservabilityFilterArgs) -> Self {
        Self {
            home: value.home_id,
            account: value.account,
            model: value.model,
            thread: value.thread,
            run: value.run,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ExportFormat {
    Json,
    Csv,
}

#[derive(Debug, Args, Default)]
struct BudgetArgs {
    #[arg(long)]
    max_total_tokens: Option<u64>,
    #[arg(long)]
    max_duration_ms: Option<u64>,
    #[arg(long)]
    max_cost_microusd: Option<u64>,
    #[arg(long)]
    max_attempts: Option<u32>,
}

impl BudgetArgs {
    fn into_budget(self) -> Option<RunBudget> {
        if self.max_total_tokens.is_none()
            && self.max_duration_ms.is_none()
            && self.max_cost_microusd.is_none()
            && self.max_attempts.is_none()
        {
            None
        } else {
            Some(RunBudget {
                max_total_tokens: self.max_total_tokens,
                max_duration_ms: self.max_duration_ms,
                max_cost_microusd: self.max_cost_microusd,
                max_attempts: self.max_attempts,
            })
        }
    }
}

#[derive(Debug, Args)]
struct IdentityArgs {
    #[arg(long)]
    home_id: String,
    #[arg(long)]
    home_alias: Option<String>,
    #[arg(long)]
    account: Option<String>,
    #[arg(long)]
    provider: Option<String>,
    #[arg(long)]
    model: String,
    #[arg(long)]
    agent_role: Option<String>,
}

impl From<IdentityArgs> for ExecutionIdentity {
    fn from(value: IdentityArgs) -> Self {
        Self {
            home_id: Some(value.home_id),
            home_alias: value.home_alias,
            account_id: value.account,
            provider: value.provider,
            model: Some(value.model),
            agent_role: value.agent_role,
        }
    }
}

#[derive(Debug, Args)]
struct TerminalUsageArgs {
    #[arg(long)]
    input_tokens: u64,
    #[arg(long)]
    output_tokens: u64,
    #[arg(long, default_value_t = 0)]
    cached_input_tokens: u64,
    #[arg(long, default_value_t = 0)]
    cache_hits: u64,
    #[arg(long, default_value_t = 0)]
    cache_misses: u64,
    #[arg(long)]
    duration_ms: u64,
    #[arg(long)]
    estimated_cost_microusd: u64,
    #[arg(long, default_value_t = 0)]
    retries: u64,
}

impl From<TerminalUsageArgs> for UsageDelta {
    fn from(value: TerminalUsageArgs) -> Self {
        Self {
            input_tokens: value.input_tokens,
            output_tokens: value.output_tokens,
            cached_input_tokens: value.cached_input_tokens,
            cache_hits: value.cache_hits,
            cache_misses: value.cache_misses,
            duration_ms: value.duration_ms,
            estimated_cost_microusd: value.estimated_cost_microusd,
            retries: value.retries,
        }
    }
}

#[derive(Debug, Args, Default)]
struct ToolUsageArgs {
    #[arg(long, default_value_t = 0)]
    input_tokens: u64,
    #[arg(long, default_value_t = 0)]
    output_tokens: u64,
    #[arg(long, default_value_t = 0)]
    cached_input_tokens: u64,
    #[arg(long, default_value_t = 0)]
    cache_hits: u64,
    #[arg(long, default_value_t = 0)]
    cache_misses: u64,
    #[arg(long, default_value_t = 0)]
    duration_ms: u64,
    #[arg(long, default_value_t = 0)]
    estimated_cost_microusd: u64,
    #[arg(long, default_value_t = 0)]
    retries: u64,
}

impl From<ToolUsageArgs> for UsageDelta {
    fn from(value: ToolUsageArgs) -> Self {
        Self {
            input_tokens: value.input_tokens,
            output_tokens: value.output_tokens,
            cached_input_tokens: value.cached_input_tokens,
            cache_hits: value.cache_hits,
            cache_misses: value.cache_misses,
            duration_ms: value.duration_ms,
            estimated_cost_microusd: value.estimated_cost_microusd,
            retries: value.retries,
        }
    }
}

#[derive(Debug, Args)]
struct FailureArgs {
    #[arg(long)]
    failure_code: String,
    #[arg(long)]
    failure_phase: Option<String>,
    #[arg(long)]
    failure_reason: String,
    #[arg(long)]
    retryable: bool,
}

impl From<FailureArgs> for FailureInfo {
    fn from(value: FailureArgs) -> Self {
        Self {
            code: value.failure_code,
            phase: value.failure_phase,
            reason: value.failure_reason,
            retryable: value.retryable,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum FailureStatusArg {
    Failed,
    Cancelled,
    TimedOut,
}

impl From<FailureStatusArg> for ObservabilityStatus {
    fn from(value: FailureStatusArg) -> Self {
        match value {
            FailureStatusArg::Failed => Self::Failed,
            FailureStatusArg::Cancelled => Self::Cancelled,
            FailureStatusArg::TimedOut => Self::TimedOut,
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let json_errors = cli.json;
    match run(cli) {
        Ok(code) => code,
        Err(error) => {
            if json_errors {
                let envelope = json!({
                    "ok": false,
                    "schemaVersion": "codexhome.error.v1",
                    "error": {
                        "code": "command_failed",
                        "message": format!("{error:#}"),
                    }
                });
                println!(
                    "{}",
                    serde_json::to_string_pretty(&envelope)
                        .unwrap_or_else(|_| "{\"ok\":false}".to_owned())
                );
            } else {
                eprintln!("error: {error:#}");
            }
            ExitCode::from(2)
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode> {
    let Cli {
        home,
        registry,
        observability_store,
        json,
        command,
    } = cli;
    match command {
        Command::Scan => {
            let report = discovery(home)?;
            output(json, &report, || print_scan(&report))?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Inspect { target } => {
            let report = discovery(home)?;
            let home = find_home(&report, &target)?;
            if json {
                print_json(&json!({
                    "ok": true,
                    "schemaVersion": "codexhome.inspect.v1",
                    "home": home,
                    "safety": report.safety,
                }))?;
            } else {
                print_home(home);
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Doctor => {
            let report = doctor(discovery(home)?);
            output(json, &report, || print_doctor(&report))?;
            Ok(if report.ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            })
        }
        Command::Registry { command } => run_registry(command, registry, json),
        Command::Home { command } => run_home(command, registry, json),
        Command::Observe { command } => run_observe(command, observability_store, json),
        Command::Task { command } => run_task(command, observability_store, json),
        Command::Run { command } => run_agent_run(command, observability_store, json),
    }
}

fn discovery(additional_homes: Vec<PathBuf>) -> Result<DiscoveryReport> {
    let options = DiscoveryOptions::from_environment(additional_homes)
        .context("failed to prepare CODEX_HOME discovery")?;
    Ok(discover(&options))
}

fn run_registry(
    command: RegistryCommand,
    registry_path: Option<PathBuf>,
    json: bool,
) -> Result<ExitCode> {
    let store = RegistryStore::from_environment(registry_path)?;
    match command {
        RegistryCommand::List => {
            let report = store.report()?;
            output(json, &report, || print_registry(&report))?;
        }
        RegistryCommand::Show { target } => {
            let report = store.report()?;
            let home = find_registered_home(&report, &target)?;
            if json {
                print_json(&json!({
                    "ok": true,
                    "schemaVersion": "codexhome.registered-home.v1",
                    "registryPath": report.registry_path,
                    "home": home,
                    "safety": report.safety,
                }))?;
            } else {
                print_registered_home(home);
            }
        }
        RegistryCommand::Path => {
            if json {
                print_json(&json!({
                    "ok": true,
                    "schemaVersion": "codexhome.registry-path.v1",
                    "registryPath": store.path().to_string_lossy(),
                    "includesLocalPaths": true,
                }))?;
            } else {
                println!("{}", store.path().display());
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn run_home(command: HomeCommand, registry_path: Option<PathBuf>, json: bool) -> Result<ExitCode> {
    let manager = HomeManager::new(RegistryStore::from_environment(registry_path)?);
    let result = match command {
        HomeCommand::Create {
            alias,
            path,
            label,
            specialty,
            dry_run,
        } => manager.create(&alias, label.as_deref(), &path, specialty, dry_run)?,
        HomeCommand::Import {
            path,
            alias,
            label,
            specialty,
            dry_run,
        } => manager.import(&path, &alias, label.as_deref(), specialty, dry_run)?,
        HomeCommand::Clone {
            source,
            alias,
            path,
            label,
            specialty,
            copy_capabilities,
            dry_run,
        } => manager.clone_home(
            &source,
            &alias,
            label.as_deref(),
            &path,
            specialty,
            copy_capabilities,
            dry_run,
        )?,
    };
    output(json, &result, || print_mutation(&result))?;
    Ok(ExitCode::SUCCESS)
}

fn run_task(
    command: TaskCommand,
    observability_path: Option<PathBuf>,
    json: bool,
) -> Result<ExitCode> {
    let store = AgentRunStore::from_environment(observability_path)?;
    match command {
        TaskCommand::Create {
            task_id,
            label,
            kind,
        } => {
            let report = store.apply(AgentRunAction::CreateTask {
                task_id: task_id.unwrap_or_else(|| generate_id("task")),
                label,
                kind,
            })?;
            output_run_mutation(&report, json)?;
        }
        TaskCommand::List => {
            let report = store.report()?;
            if json {
                print_json(&json!({
                    "ok": true,
                    "schemaVersion": "codexhome.tasks.v1",
                    "storePath": report.store_path,
                    "tasks": report.tasks,
                    "safety": report.safety,
                }))?;
            } else {
                print_tasks(&report);
            }
        }
        TaskCommand::Show { task_id } => {
            let report = store.report()?;
            let task = report.task(&task_id)?;
            let runs = report
                .runs
                .iter()
                .filter(|run| run.task_id == task_id)
                .collect::<Vec<_>>();
            if json {
                print_json(&json!({
                    "ok": true,
                    "schemaVersion": "codexhome.task.v1",
                    "task": task,
                    "runs": runs,
                    "safety": report.safety,
                }))?;
            } else {
                println!("{} · {} · {:?}", task.task_id, task.label, task.status);
                println!("Runs: {}", task.run_ids.len());
                for run in runs {
                    print_agent_run(run);
                }
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn run_agent_run(
    command: RunCommand,
    observability_path: Option<PathBuf>,
    json: bool,
) -> Result<ExitCode> {
    let store = AgentRunStore::from_environment(observability_path)?;
    match command {
        RunCommand::Start {
            task_id,
            run_id,
            budget,
        } => apply_run_action(
            &store,
            AgentRunAction::StartRun {
                task_id,
                run_id: run_id.unwrap_or_else(|| generate_id("run")),
                budget: budget.into_budget(),
            },
            json,
        )?,
        RunCommand::List { task_id } => {
            let mut report = store.report()?;
            if let Some(task_id) = task_id {
                report.runs.retain(|run| run.task_id == task_id);
                report.tasks.retain(|task| task.task_id == task_id);
            }
            output(json, &report, || print_agent_runs(&report))?;
        }
        RunCommand::Show { run_id } => {
            let report = store.report()?;
            let run = report.run(&run_id)?;
            let task = report.task(&run.task_id)?;
            if json {
                print_json(&json!({
                    "ok": true,
                    "schemaVersion": "codexhome.agent-run.v1",
                    "task": task,
                    "run": run,
                    "safety": report.safety,
                }))?;
            } else {
                println!("{} · {}", task.task_id, task.label);
                print_agent_run(run);
            }
        }
        RunCommand::Attempt { command } => {
            let action = match command {
                AttemptCommand::Start {
                    run_id,
                    attempt_id,
                    identity,
                    route_reason,
                } => AgentRunAction::StartAttempt {
                    run_id,
                    attempt_id: attempt_id.unwrap_or_else(|| generate_id("attempt")),
                    identity: identity.into(),
                    transition: AttemptTransition {
                        kind: AttemptTransitionKind::Initial,
                        from_attempt_id: None,
                    },
                    route_reason,
                },
                AttemptCommand::Retry {
                    run_id,
                    from_attempt_id,
                    attempt_id,
                    identity,
                    route_reason,
                } => AgentRunAction::StartAttempt {
                    run_id,
                    attempt_id: attempt_id.unwrap_or_else(|| generate_id("attempt")),
                    identity: identity.into(),
                    transition: AttemptTransition {
                        kind: AttemptTransitionKind::Retry,
                        from_attempt_id: Some(from_attempt_id),
                    },
                    route_reason: Some(route_reason),
                },
                AttemptCommand::Migrate {
                    run_id,
                    from_attempt_id,
                    attempt_id,
                    identity,
                    route_reason,
                } => AgentRunAction::StartAttempt {
                    run_id,
                    attempt_id: attempt_id.unwrap_or_else(|| generate_id("attempt")),
                    identity: identity.into(),
                    transition: AttemptTransition {
                        kind: AttemptTransitionKind::Migration,
                        from_attempt_id: Some(from_attempt_id),
                    },
                    route_reason: Some(route_reason),
                },
                AttemptCommand::Complete {
                    run_id,
                    attempt_id,
                    thread_id,
                    usage,
                } => AgentRunAction::CompleteAttempt {
                    run_id,
                    attempt_id,
                    thread_id,
                    usage: usage.into(),
                },
                AttemptCommand::Fail {
                    run_id,
                    attempt_id,
                    thread_id,
                    status,
                    usage,
                    failure,
                } => AgentRunAction::FailAttempt {
                    run_id,
                    attempt_id,
                    thread_id,
                    status: status.into(),
                    usage: usage.into(),
                    failure: failure.into(),
                },
            };
            apply_run_action(&store, action, json)?;
        }
        RunCommand::Thread { command } => {
            let ThreadCommand::Link {
                run_id,
                thread_id,
                attempt_id,
                parent_thread_id,
                fork_id,
            } = command;
            apply_run_action(
                &store,
                AgentRunAction::LinkThread {
                    run_id,
                    attempt_id,
                    thread_id,
                    parent_thread_id,
                    fork_id,
                },
                json,
            )?;
        }
        RunCommand::Tool { command } => {
            let ToolCommand::Record {
                run_id,
                attempt_id,
                thread_id,
                tool_name,
                usage,
            } = command;
            apply_run_action(
                &store,
                AgentRunAction::RecordTool {
                    run_id,
                    attempt_id,
                    thread_id,
                    tool_name,
                    usage: usage.into(),
                },
                json,
            )?;
        }
        RunCommand::Artifact { command } => {
            let ArtifactCommand::Record {
                run_id,
                attempt_id,
                thread_id,
                artifact_id,
                artifact_kind,
            } = command;
            apply_run_action(
                &store,
                AgentRunAction::RecordArtifact {
                    run_id,
                    attempt_id,
                    thread_id,
                    artifact_id: artifact_id.unwrap_or_else(|| generate_id("artifact")),
                    artifact_kind,
                },
                json,
            )?;
        }
        RunCommand::Verification { command } => {
            let VerificationCommand::Record {
                run_id,
                attempt_id,
                thread_id,
                verification_id,
                verification_kind,
                target_artifact_id,
                succeeded,
                duration_ms,
            } = command;
            apply_run_action(
                &store,
                AgentRunAction::RecordVerification {
                    run_id,
                    attempt_id,
                    thread_id,
                    verification_id: verification_id.unwrap_or_else(|| generate_id("verification")),
                    verification_kind,
                    target_artifact_id,
                    succeeded,
                    duration_ms,
                },
                json,
            )?;
        }
        RunCommand::Complete {
            run_id,
            final_artifact_ids,
        } => apply_run_action(
            &store,
            AgentRunAction::CompleteRun {
                run_id,
                final_artifact_ids,
            },
            json,
        )?,
        RunCommand::Fail {
            run_id,
            status,
            failure,
        } => apply_run_action(
            &store,
            AgentRunAction::FailRun {
                run_id,
                status: status.into(),
                failure: failure.into(),
            },
            json,
        )?,
        RunCommand::Path => {
            if json {
                print_json(&json!({
                    "ok": true,
                    "schemaVersion": "codexhome.agent-run-path.v1",
                    "storePath": store.path().to_string_lossy(),
                    "includesLocalPaths": true,
                }))?;
            } else {
                println!("{}", store.path().display());
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn apply_run_action(store: &AgentRunStore, action: AgentRunAction, json: bool) -> Result<()> {
    let report = store.apply(action)?;
    output_run_mutation(&report, json)
}

fn output_run_mutation(report: &AgentRunMutationReport, json: bool) -> Result<()> {
    output(json, report, || {
        println!(
            "{} · task={} · run={} · attempt={}",
            report.action,
            report.task_id,
            report.run_id.as_deref().unwrap_or("-"),
            report.attempt_id.as_deref().unwrap_or("-")
        );
        if let Some(run) = &report.run {
            print_agent_run(run);
        }
    })
}

fn run_observe(
    command: ObserveCommand,
    store_path: Option<PathBuf>,
    json: bool,
) -> Result<ExitCode> {
    let store = ObservabilityStore::from_environment(store_path)?;
    match command {
        ObserveCommand::Record { input } => {
            let contents = read_observability_input(&input)?;
            let events = parse_observability_events(&contents)?;
            let report = store.append(&events)?;
            output(json, &report, || {
                println!(
                    "Recorded {} event(s); store now contains {}.",
                    report.appended, report.total_events
                );
                println!("{}", report.store_path);
            })?;
        }
        ObserveCommand::Summary { filter } => {
            let report = store.summary(&filter.into())?;
            output(json, &report, || print_observability_summary(&report))?;
        }
        ObserveCommand::Export {
            filter,
            format,
            output: output_path,
        } => {
            let export = store.export(&filter.into())?;
            if let Some(parent) = output_path
                .parent()
                .filter(|path| !path.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
            let format_name = match format {
                ExportFormat::Json => {
                    let contents = serde_json::to_string_pretty(&export)
                        .context("failed to serialize observability export")?;
                    fs::write(&output_path, format!("{contents}\n"))
                        .with_context(|| format!("failed to write {}", output_path.display()))?;
                    "json"
                }
                ExportFormat::Csv => {
                    fs::write(&output_path, observability_events_csv(&export.events))
                        .with_context(|| format!("failed to write {}", output_path.display()))?;
                    "csv"
                }
            };
            let report = json!({
                "ok": true,
                "schemaVersion": "codexhome.observability-export-report.v1",
                "format": format_name,
                "outputPath": output_path.to_string_lossy(),
                "eventCount": export.events.len(),
                "safety": export.safety,
            });
            if json {
                print_json(&report)?;
            } else {
                println!(
                    "Exported {} event(s) as {} to {}.",
                    export.events.len(),
                    format_name,
                    output_path.display()
                );
            }
        }
        ObserveCommand::Verify => {
            let report = store.verify()?;
            output(json, &report, || {
                println!(
                    "Observability store verified: {} event(s), {} task(s), {} run(s), {} attempt(s), {} thread(s).",
                    report.event_count,
                    report.task_count,
                    report.run_count,
                    report.attempt_count,
                    report.thread_count
                );
                println!("{}", report.store_path);
            })?;
        }
        ObserveCommand::Path => {
            if json {
                print_json(&json!({
                    "ok": true,
                    "schemaVersion": "codexhome.observability-path.v1",
                    "storePath": store.path().to_string_lossy(),
                    "includesLocalPaths": true,
                }))?;
            } else {
                println!("{}", store.path().display());
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn read_observability_input(input: &str) -> Result<String> {
    if input == "-" {
        let mut contents = String::new();
        io::stdin()
            .read_to_string(&mut contents)
            .context("failed to read observability events from stdin")?;
        Ok(contents)
    } else {
        fs::read_to_string(input)
            .with_context(|| format!("failed to read observability input {input}"))
    }
}

fn output(value_is_json: bool, value: &impl serde::Serialize, human: impl FnOnce()) -> Result<()> {
    if value_is_json {
        print_json(value)
    } else {
        human();
        Ok(())
    }
}

fn find_home<'a>(report: &'a DiscoveryReport, target: &str) -> Result<&'a CodexHomeSummary> {
    let matches = report
        .homes
        .iter()
        .filter(|home| home.matches(target))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [home] => Ok(home),
        [] => {
            if report.homes.is_empty() {
                bail!("no Codex Homes were discovered; pass one explicitly with --home <PATH>");
            }
            let available = report
                .homes
                .iter()
                .map(|home| format!("{} ({})", home.label, home.id))
                .collect::<Vec<_>>()
                .join(", ");
            bail!("Home '{target}' was not found; available Homes: {available}")
        }
        _ => {
            let ids = matches
                .iter()
                .map(|home| home.id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            bail!("Home '{target}' is ambiguous; use one of these IDs: {ids}")
        }
    }
}

fn find_registered_home<'a>(
    report: &'a RegistryReport,
    target: &str,
) -> Result<&'a RegisteredHomeView> {
    let matches = report
        .homes
        .iter()
        .filter(|home| home.entry.matches(target))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [home] => Ok(home),
        [] => bail!(
            "registered Home '{target}' was not found; run `codexhome registry list` or import it first"
        ),
        _ => {
            let aliases = matches
                .iter()
                .map(|home| home.entry.alias.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            bail!("registered Home '{target}' is ambiguous; use one of: {aliases}")
        }
    }
}

fn print_json(value: &impl serde::Serialize) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(value).context("failed to serialize JSON output")?
    );
    Ok(())
}

fn print_scan(report: &DiscoveryReport) {
    println!("CodexHome Manager · {} Home(s)", report.homes.len());
    if report.homes.is_empty() {
        println!("No Codex Homes discovered.");
        println!("Try: codexhome --home /path/to/CODEX_HOME scan");
    } else {
        println!();
        for home in &report.homes {
            println!(
                "{}  {}  provider={}  skills={}  mcp={}",
                home.id,
                home.label,
                display(home.provider.as_deref()),
                home.skill_count,
                home.mcp_server_count
            );
            println!("  {}", home.path);
        }
    }
    print_warnings(&report.warnings);
    println!();
    println!("Safety: credential contents were not read or printed.");
}

fn print_registry(report: &RegistryReport) {
    println!(
        "CodexHome Registry · {} Home(s) · revision {}",
        report.homes.len(),
        report.revision
    );
    println!("{}", report.registry_path);
    if report.homes.is_empty() {
        println!();
        println!("No registered Homes.");
        println!("Try: codexhome home import /path/to/home --alias @research");
        return;
    }
    println!();
    for home in &report.homes {
        let state = if home.available { "ready" } else { "missing" };
        let specialties = if home.entry.specialties.is_empty() {
            "general".to_owned()
        } else {
            home.entry.specialties.join(",")
        };
        println!(
            "{}  {}  [{}]  specialties={}",
            home.entry.alias, home.entry.label, state, specialties
        );
        println!("  {}", home.entry.path);
        if let Some(summary) = &home.summary {
            println!(
                "  provider={} skills={} mcp={}",
                display(summary.provider.as_deref()),
                summary.skill_count,
                summary.mcp_server_count
            );
        }
        for issue in &home.issues {
            println!("  warning: {issue}");
        }
    }
}

fn print_tasks(report: &AgentRunReport) {
    println!(
        "CodexHome Tasks · {} task(s) · {} run(s)",
        report.tasks.len(),
        report.runs.len()
    );
    println!("{}", report.store_path);
    for task in &report.tasks {
        println!(
            "{} · {:?} · {} · runs={}",
            task.task_id,
            task.status,
            task.label,
            task.run_ids.len()
        );
    }
}

fn print_agent_runs(report: &AgentRunReport) {
    println!(
        "CodexHome Agent Runs · {} run(s) · {} task(s)",
        report.runs.len(),
        report.tasks.len()
    );
    println!("{}", report.store_path);
    for run in &report.runs {
        print_agent_run(run);
    }
}

fn print_agent_run(run: &AgentRunView) {
    println!(
        "{} · {:?} · attempts={} · tokens={} · duration={}ms · cost={} microUSD",
        run.run_id,
        run.status,
        run.attempts.len(),
        run.consumption.total_tokens,
        run.consumption.duration_ms,
        run.consumption.estimated_cost_microusd
    );
    println!(
        "  failure cost: {} tokens · {}ms · {} microUSD",
        run.consumption.failed_attempt_tokens,
        run.consumption.failed_attempt_duration_ms,
        run.consumption.failed_attempt_cost_microusd
    );
    println!(
        "  budget: {} · recovery: {} ({})",
        if run.budget_state.within_budget {
            "available".to_owned()
        } else {
            format!(
                "exhausted:{}",
                run.budget_state.exhausted_dimensions.join(",")
            )
        },
        run.recovery.recommended_action,
        run.recovery.reason
    );
    println!(
        "  threads={} tools={} artifacts={} verifications={}",
        run.thread_ids.len(),
        run.tool_calls,
        run.artifacts.len(),
        run.verifications.len()
    );
    for attempt in &run.attempts {
        println!(
            "  attempt {} · {:?} · home={} · model={} · {} tokens · {}ms",
            attempt.attempt_id,
            attempt.status,
            display(attempt.identity.home_id.as_deref()),
            display(attempt.identity.model.as_deref()),
            attempt
                .usage
                .input_tokens
                .saturating_add(attempt.usage.output_tokens),
            attempt.usage.duration_ms
        );
    }
}

fn print_observability_summary(report: &ObservabilitySummary) {
    let totals = &report.totals;
    println!(
        "CodexHome Observability · {} event(s) · {} task(s) · {} run(s) · {} attempt(s)",
        report.event_count, totals.tasks, totals.runs, totals.attempts
    );
    println!("{}", report.store_path);
    println!();
    println!(
        "Tokens: {} input + {} output = {} total ({} cached input)",
        totals.input_tokens, totals.output_tokens, totals.total_tokens, totals.cached_input_tokens
    );
    println!(
        "Duration: {} ms · retries: {} · estimated cost: {} microUSD",
        totals.duration_ms, totals.retries, totals.estimated_cost_microusd
    );
    println!(
        "Cache hit rate: {:.2}% · attempt failure rate: {:.2}%",
        f64::from(totals.cache_hit_rate_basis_points) / 100.0,
        f64::from(totals.failure_rate_basis_points) / 100.0
    );
    println!(
        "Tool calls: {} · artifacts: {} · verifications: {} · threads: {}",
        totals.tool_calls, totals.artifacts, totals.verifications, totals.threads
    );
    if !report.latest_home_health.is_empty() {
        println!();
        println!("Latest Home health:");
        for health in &report.latest_home_health {
            println!(
                "  {} · {:?} · service={} · auth={}",
                health.home_id,
                health.status,
                if health.snapshot.service_reachable {
                    "reachable"
                } else {
                    "unreachable"
                },
                health
                    .snapshot
                    .auth_valid
                    .map(|valid| if valid { "valid" } else { "invalid" })
                    .unwrap_or("unknown")
            );
        }
    }
}

fn print_registered_home(home: &RegisteredHomeView) {
    println!("{} · {}", home.entry.alias, home.entry.label);
    println!("  Registry ID:      {}", home.entry.id);
    println!("  Path:             {}", home.entry.path);
    println!("  Origin:           {:?}", home.entry.origin);
    println!(
        "  Specialties:      {}",
        if home.entry.specialties.is_empty() {
            "general".to_owned()
        } else {
            home.entry.specialties.join(", ")
        }
    );
    println!(
        "  Availability:     {}",
        if home.available { "ready" } else { "missing" }
    );
    if let Some(summary) = &home.summary {
        println!();
        print_home(summary);
    }
    for issue in &home.issues {
        println!("  Warning: {issue}");
    }
}

fn print_mutation(result: &HomeMutationResult) {
    let mode = if result.dry_run { "DRY RUN" } else { "DONE" };
    println!(
        "[{mode}] {} {} · {}",
        result.action, result.entry.alias, result.entry.label
    );
    println!("  Path:                {}", result.entry.path);
    println!(
        "  Specialties:         {}",
        if result.entry.specialties.is_empty() {
            "general".to_owned()
        } else {
            result.entry.specialties.join(", ")
        }
    );
    println!("  Registry revision:   {}", result.registry_revision);
    println!(
        "  Files copied:        {}",
        result.copy_summary.files_copied
    );
    println!(
        "  Files skipped:       {}",
        result.copy_summary.files_skipped
    );
    println!();
    println!("Plan:");
    for action in &result.planned_actions {
        println!("  - {action}");
    }
    if !result.warnings.is_empty() {
        println!();
        println!("Warnings:");
        for warning in &result.warnings {
            println!("  - {warning}");
        }
    }
}

fn print_home(home: &CodexHomeSummary) {
    println!("{}", home.label);
    println!("  ID:               {}", home.id);
    println!("  Path:             {}", home.path);
    println!("  Source:           {}", home.source);
    println!("  Provider:         {}", display(home.provider.as_deref()));
    println!("  Model:            {}", display(home.model.as_deref()));
    println!(
        "  Endpoint host:    {}",
        display(home.endpoint_host.as_deref())
    );
    println!(
        "  Reasoning effort: {}",
        display(home.reasoning_effort.as_deref())
    );
    println!(
        "  Approval policy:  {}",
        display(home.approval_policy.as_deref())
    );
    println!(
        "  Sandbox mode:     {}",
        display(home.sandbox_mode.as_deref())
    );
    println!(
        "  Web search:       {}",
        display(home.web_search.as_deref())
    );
    println!(
        "  Config:           {}",
        file_state(home.has_config, home.config_valid)
    );
    println!("  Authentication:   {}", present_state(home.has_auth));
    println!("  Skills:           {}", home.skill_count);
    println!("  Plugins:          {}", home.plugin_count);
    println!("  MCP servers:      {}", home.mcp_server_count);
    println!("  Enabled features: {}", home.enabled_feature_count);
    println!("  Rules:            {}", home.rule_count);
    println!("  Hooks:            {}", home.hook_count);
    let issues = home.issues();
    if !issues.is_empty() {
        println!("  Issues:           {}", issues.join("; "));
    }
    println!();
    println!("Credential contents were not read or printed.");
}

fn print_doctor(report: &codexhome_core::DoctorReport) {
    let status = if report.ok {
        "healthy"
    } else {
        "needs attention"
    };
    println!(
        "CodexHome Doctor · {status} · {}/{} healthy",
        report.healthy_home_count, report.home_count
    );
    if report.checks.is_empty() {
        println!("No Codex Homes discovered.");
    } else {
        println!();
        for check in &report.checks {
            let marker = if check.ok { "OK" } else { "WARN" };
            println!("[{marker}] {} ({})", check.label, check.home_id);
            for issue in &check.issues {
                println!("  - {issue}");
            }
        }
    }
    print_warnings(&report.warnings);
}

fn print_warnings(warnings: &[codexhome_core::DiscoveryWarning]) {
    if warnings.is_empty() {
        return;
    }
    println!();
    println!("Warnings:");
    for warning in warnings {
        println!("  - [{}] {}", warning.code, warning.message);
    }
}

fn display(value: Option<&str>) -> &str {
    value.unwrap_or("—")
}

fn present_state(present: bool) -> &'static str {
    if present {
        "present (contents not read)"
    } else {
        "missing"
    }
}

fn file_state(present: bool, valid: bool) -> &'static str {
    match (present, valid) {
        (true, true) => "valid",
        (true, false) => "invalid",
        (false, _) => "missing",
    }
}
