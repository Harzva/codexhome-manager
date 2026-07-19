use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use codexhome_core::{discover, doctor, CodexHomeSummary, DiscoveryOptions, DiscoveryReport};
use serde_json::json;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Debug, Parser)]
#[command(
    name = "codexhome",
    version,
    about = "Discover and manage specialized Codex Homes",
    long_about = "Discover and inspect multiple CODEX_HOME directories as isolated Skill Spaces and Agent Households.\n\nCredential contents are never included in command output."
)]
struct Cli {
    /// Add a CODEX_HOME candidate without changing the active shell environment.
    #[arg(long, global = true, value_name = "PATH")]
    home: Vec<PathBuf>,

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

    /// Inspect one Home by ID, label, provider, or absolute path.
    Inspect {
        /// ID, label, provider, or path returned by `codexhome scan`.
        target: String,
    },

    /// Check discovered Homes for common configuration problems.
    Doctor,
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::from(2)
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode> {
    let options = DiscoveryOptions::from_environment(cli.home)
        .context("failed to prepare CODEX_HOME discovery")?;
    let report = discover(&options);

    match cli.command {
        Command::Scan => {
            if cli.json {
                print_json(&report)?;
            } else {
                print_scan(&report);
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Inspect { target } => {
            let home = find_home(&report, &target)?;
            if cli.json {
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
            let report = doctor(report);
            if cli.json {
                print_json(&report)?;
            } else {
                print_doctor(&report);
            }
            Ok(if report.ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            })
        }
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
