use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use codexhome_core::{
    discover, doctor, observability_events_csv, parse_observability_events, CodexHomeSummary,
    DiscoveryOptions, DiscoveryReport, HomeManager, HomeMutationResult, ObservabilityFilter,
    ObservabilityStore, ObservabilitySummary, RegisteredHomeView, RegistryReport, RegistryStore,
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
    after_help = "Examples:\n  codexhome scan\n  codexhome registry list\n  codexhome home import /path/to/home --alias @research --specialty papers\n  codexhome observe record events.jsonl\n  codexhome observe summary --home-id @research --json"
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
