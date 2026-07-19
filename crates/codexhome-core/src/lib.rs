use anyhow::{Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

mod home;
mod registry;

pub use home::{CopySummary, HomeManager, HomeMutationResult};
pub use registry::{
    normalize_alias, normalize_specialties, RegisteredHomeView, Registry, RegistryEntry,
    RegistryOrigin, RegistryReport, RegistryStore,
};

pub const DISCOVERY_SCHEMA_VERSION: &str = "codexhome.discovery.v1";
pub const DOCTOR_SCHEMA_VERSION: &str = "codexhome.doctor.v1";

#[derive(Debug, Clone)]
pub struct DiscoveryOptions {
    pub user_home: PathBuf,
    pub explicit_home: Option<PathBuf>,
    pub additional_homes: Vec<PathBuf>,
    pub include_known_locations: bool,
}

impl DiscoveryOptions {
    pub fn from_environment(additional_homes: Vec<PathBuf>) -> Result<Self> {
        let user_home = user_home().context("cannot discover the current user home directory")?;
        let explicit_home = env::var_os("CODEX_HOME").map(PathBuf::from);
        let mut homes = additional_homes;
        if let Some(paths) = env::var_os("CODEXHOME_PATHS") {
            homes.extend(env::split_paths(&paths));
        }
        Ok(Self {
            user_home,
            explicit_home,
            additional_homes: homes,
            include_known_locations: true,
        })
    }

    #[cfg(test)]
    fn for_test(user_home: PathBuf) -> Self {
        Self {
            user_home,
            explicit_home: None,
            additional_homes: Vec::new(),
            include_known_locations: true,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryReport {
    pub ok: bool,
    pub schema_version: &'static str,
    pub homes: Vec<CodexHomeSummary>,
    pub warnings: Vec<DiscoveryWarning>,
    pub safety: SafetySummary,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryWarning {
    pub code: &'static str,
    pub home_id: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SafetySummary {
    pub secrets_redacted: bool,
    pub includes_secrets: bool,
    pub reads_auth_contents: bool,
    pub includes_local_paths: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexHomeSummary {
    pub id: String,
    pub label: String,
    pub path: String,
    pub source: String,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub endpoint_host: Option<String>,
    pub reasoning_effort: Option<String>,
    pub approval_policy: Option<String>,
    pub sandbox_mode: Option<String>,
    pub web_search: Option<String>,
    pub has_auth: bool,
    pub has_config: bool,
    pub config_valid: bool,
    pub skill_count: usize,
    pub plugin_count: usize,
    pub mcp_server_count: usize,
    pub enabled_feature_count: usize,
    pub rule_count: usize,
    pub hook_count: usize,
}

impl CodexHomeSummary {
    pub fn matches(&self, query: &str) -> bool {
        self.id == query
            || self.path == query
            || self.label.eq_ignore_ascii_case(query)
            || self
                .provider
                .as_deref()
                .is_some_and(|value| value.eq_ignore_ascii_case(query))
    }

    pub fn issues(&self) -> Vec<&'static str> {
        let mut issues = Vec::new();
        if !self.has_config {
            issues.push("config.toml is missing");
        } else if !self.config_valid {
            issues.push("config.toml is invalid");
        }
        if !self.has_auth {
            issues.push("authentication file is missing");
        }
        issues
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorReport {
    pub ok: bool,
    pub schema_version: &'static str,
    pub home_count: usize,
    pub healthy_home_count: usize,
    pub checks: Vec<DoctorCheck>,
    pub warnings: Vec<DiscoveryWarning>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorCheck {
    pub home_id: String,
    pub label: String,
    pub ok: bool,
    pub issues: Vec<String>,
}

pub fn discover(options: &DiscoveryOptions) -> DiscoveryReport {
    let mut candidates = BTreeMap::<PathBuf, String>::new();
    add_candidate(
        &mut candidates,
        options.user_home.join(".codex"),
        "default",
        &options.user_home,
    );
    if let Some(path) = &options.explicit_home {
        add_candidate(
            &mut candidates,
            path.clone(),
            "environment",
            &options.user_home,
        );
    }
    for path in &options.additional_homes {
        add_candidate(
            &mut candidates,
            path.clone(),
            "configured",
            &options.user_home,
        );
    }
    if options.include_known_locations {
        add_group_children(
            &mut candidates,
            &options.user_home.join(".codex-managed-agent/accounts"),
            "managed-agent",
            &options.user_home,
        );
        add_group_children(
            &mut candidates,
            &options
                .user_home
                .join("Library/Application Support/open-codex-launcher/profile-homes"),
            "open-codex-launcher",
            &options.user_home,
        );
        add_group_children(
            &mut candidates,
            &options
                .user_home
                .join("Library/Application Support/codexuse-desktop/profile-homes"),
            "codexuse-desktop",
            &options.user_home,
        );
    }

    let mut homes = Vec::new();
    let mut warnings = Vec::new();
    for (path, source) in candidates {
        if !looks_like_codex_home(&path) {
            if matches!(source.as_str(), "environment" | "configured") {
                let (code, reason) = if path.is_dir() {
                    ("not_codex_home", "is not a recognizable Codex Home")
                } else {
                    ("home_not_found", "does not exist or is not a directory")
                };
                warnings.push(DiscoveryWarning {
                    code,
                    home_id: None,
                    message: format!("{} {reason}", path.to_string_lossy()),
                });
            }
            continue;
        }
        let (home, warning) = inspect_home(&path, &source);
        if let Some(warning) = warning {
            warnings.push(warning);
        }
        homes.push(home);
    }
    homes.sort_by(|left, right| {
        source_rank(&left.source)
            .cmp(&source_rank(&right.source))
            .then(left.label.cmp(&right.label))
    });

    DiscoveryReport {
        ok: true,
        schema_version: DISCOVERY_SCHEMA_VERSION,
        homes,
        warnings,
        safety: SafetySummary {
            secrets_redacted: true,
            includes_secrets: false,
            reads_auth_contents: false,
            includes_local_paths: true,
        },
    }
}

pub fn doctor(discovery: DiscoveryReport) -> DoctorReport {
    let checks = discovery
        .homes
        .iter()
        .map(|home| {
            let issues = home
                .issues()
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>();
            DoctorCheck {
                home_id: home.id.clone(),
                label: home.label.clone(),
                ok: issues.is_empty(),
                issues,
            }
        })
        .collect::<Vec<_>>();
    let healthy_home_count = checks.iter().filter(|check| check.ok).count();
    DoctorReport {
        ok: !checks.is_empty()
            && healthy_home_count == checks.len()
            && discovery.warnings.is_empty(),
        schema_version: DOCTOR_SCHEMA_VERSION,
        home_count: checks.len(),
        healthy_home_count,
        checks,
        warnings: discovery.warnings,
    }
}

pub fn inspect_home_path(
    path: &Path,
    source: &str,
) -> Result<(CodexHomeSummary, Option<DiscoveryWarning>)> {
    if !looks_like_codex_home(path) {
        anyhow::bail!("{} is not a recognizable Codex Home", path.display());
    }
    Ok(inspect_home(path, source))
}

fn inspect_home(path: &Path, source: &str) -> (CodexHomeSummary, Option<DiscoveryWarning>) {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let id = stable_id(&canonical);
    let config_path = canonical.join("config.toml");
    let has_config = config_path.is_file();
    let has_auth = canonical.join("auth.json").is_file();
    let mut config_valid = false;
    let mut model = None;
    let mut provider = None;
    let mut endpoint_host = None;
    let mut reasoning_effort = None;
    let mut approval_policy = None;
    let mut sandbox_mode = None;
    let mut web_search = None;
    let mut mcp_server_count = 0;
    let mut enabled_feature_count = 0;
    let mut configured_hook_count = 0;
    let mut warning = None;

    if has_config {
        match fs::read_to_string(&config_path)
            .ok()
            .and_then(|content| content.parse::<toml::Table>().ok())
            .map(toml::Value::Table)
        {
            Some(value) => {
                config_valid = true;
                model = string_setting(&value, "model");
                reasoning_effort = string_setting(&value, "model_reasoning_effort");
                approval_policy = string_setting(&value, "approval_policy");
                sandbox_mode = string_setting(&value, "sandbox_mode");
                web_search = scalar_setting(&value, "web_search");
                let provider_key = value
                    .get("model_provider")
                    .and_then(toml::Value::as_str)
                    .unwrap_or("openai");
                provider = value
                    .get("model_providers")
                    .and_then(|item| item.get(provider_key))
                    .and_then(|item| item.get("name"))
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned)
                    .or_else(|| Some(provider_key.to_owned()));
                endpoint_host = value
                    .get("model_providers")
                    .and_then(|item| item.get(provider_key))
                    .and_then(|item| item.get("base_url"))
                    .and_then(toml::Value::as_str)
                    .and_then(url_host);
                mcp_server_count = table_len(&value, "mcp_servers");
                configured_hook_count = table_len(&value, "hooks");
                enabled_feature_count = value
                    .get("features")
                    .and_then(toml::Value::as_table)
                    .map(|items| {
                        items
                            .values()
                            .filter(|item| item.as_bool().unwrap_or(false))
                            .count()
                    })
                    .unwrap_or(0);
            }
            None => {
                warning = Some(DiscoveryWarning {
                    code: "invalid_config",
                    home_id: Some(id.clone()),
                    message: format!(
                        "{} contains an unreadable or invalid config.toml",
                        display_label(&canonical)
                    ),
                });
            }
        }
    }

    let leaf = canonical
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("codex-home");
    let label = home_label(source, leaf, provider.as_deref());
    let hook_count = configured_hook_count + directory_entry_count(&canonical.join("hooks"), None);

    (
        CodexHomeSummary {
            id,
            label,
            path: canonical.to_string_lossy().into_owned(),
            source: source.to_owned(),
            model,
            provider,
            endpoint_host,
            reasoning_effort,
            approval_policy,
            sandbox_mode,
            web_search,
            has_auth,
            has_config,
            config_valid,
            skill_count: directory_entry_count(&canonical.join("skills"), Some(true)),
            plugin_count: directory_entry_count(&canonical.join("plugins"), Some(true)),
            mcp_server_count,
            enabled_feature_count,
            rule_count: directory_entry_count(&canonical.join("rules"), Some(false)),
            hook_count,
        },
        warning,
    )
}

fn add_group_children(
    candidates: &mut BTreeMap<PathBuf, String>,
    root: &Path,
    source: &str,
    user_home: &Path,
) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            add_candidate(candidates, entry.path(), source, user_home);
        }
    }
}

fn add_candidate(
    candidates: &mut BTreeMap<PathBuf, String>,
    path: PathBuf,
    source: &str,
    user_home: &Path,
) {
    let expanded = expand_home(&path, user_home);
    let key = expanded.canonicalize().unwrap_or(expanded);
    candidates.entry(key).or_insert_with(|| source.to_owned());
}

fn looks_like_codex_home(path: &Path) -> bool {
    path.is_dir()
        && (path.join("config.toml").is_file()
            || path.join("auth.json").is_file()
            || path.join("state_5.sqlite").is_file()
            || path.join("sessions").is_dir()
            || path.join("skills").is_dir())
}

pub(crate) fn user_home() -> Option<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

pub(crate) fn expand_home(path: &Path, user_home: &Path) -> PathBuf {
    let raw = path.to_string_lossy();
    if raw == "~" {
        return user_home.to_path_buf();
    }
    if let Some(rest) = raw.strip_prefix("~/").or_else(|| raw.strip_prefix("~\\")) {
        return user_home.join(rest);
    }
    path.to_path_buf()
}

pub(crate) fn stable_id(path: &Path) -> String {
    let mut digest = Sha256::new();
    digest.update(path.to_string_lossy().as_bytes());
    format!("{:x}", digest.finalize())[..12].to_owned()
}

fn source_rank(source: &str) -> u8 {
    match source {
        "default" => 0,
        "environment" => 1,
        "configured" => 2,
        "open-codex-launcher" => 3,
        "codexuse-desktop" => 4,
        "managed-agent" => 5,
        _ => 6,
    }
}

fn home_label(source: &str, leaf: &str, provider: Option<&str>) -> String {
    match source {
        "default" => "Default Codex".to_owned(),
        "environment" => "Environment Codex".to_owned(),
        "open-codex-launcher" if leaf.starts_with("apikey-") => {
            format!("API · {}", provider.unwrap_or("Custom"))
        }
        "open-codex-launcher" if leaf.starts_with("auth-") => "Subscription Profile".to_owned(),
        "open-codex-launcher" => format!("Launcher · {}", safe_leaf_label(leaf)),
        "codexuse-desktop" => format!("CodexUse · {}", safe_leaf_label(leaf)),
        "managed-agent" => format!("Managed · {}", safe_leaf_label(leaf)),
        "configured" => format!("Custom · {}", safe_leaf_label(leaf)),
        _ => format!("Codex · {}", safe_leaf_label(leaf)),
    }
}

fn safe_leaf_label(value: &str) -> String {
    if value.len() >= 7 && value.chars().all(|character| character.is_ascii_digit()) {
        let suffix = value.chars().rev().take(4).collect::<String>();
        return format!("Account •••{}", suffix.chars().rev().collect::<String>());
    }
    value.to_owned()
}

fn display_label(path: &Path) -> String {
    path.file_name()
        .and_then(|value| value.to_str())
        .map(safe_leaf_label)
        .unwrap_or_else(|| "CODEX_HOME".to_owned())
}

fn url_host(value: &str) -> Option<String> {
    let without_scheme = value
        .split_once("://")
        .map(|(_, tail)| tail)
        .unwrap_or(value);
    let without_credentials = without_scheme
        .rsplit_once('@')
        .map(|(_, tail)| tail)
        .unwrap_or(without_scheme);
    without_credentials
        .split(['/', ':'])
        .next()
        .filter(|host| !host.is_empty())
        .map(str::to_owned)
}

fn string_setting(value: &toml::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(toml::Value::as_str)
        .map(str::to_owned)
}

fn scalar_setting(value: &toml::Value, key: &str) -> Option<String> {
    let item = value.get(key)?;
    if let Some(value) = item.as_str() {
        return Some(value.to_owned());
    }
    item.as_bool().map(|value| value.to_string())
}

fn table_len(value: &toml::Value, key: &str) -> usize {
    value
        .get(key)
        .and_then(toml::Value::as_table)
        .map(|items| items.len())
        .unwrap_or(0)
}

fn directory_entry_count(path: &Path, directory_filter: Option<bool>) -> usize {
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|entry| {
            let Ok(kind) = entry.file_type() else {
                return false;
            };
            match directory_filter {
                Some(true) => kind.is_dir(),
                Some(false) => kind.is_file(),
                None => kind.is_dir() || kind.is_file(),
            }
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn discovers_safe_capabilities_without_serializing_secrets() {
        let temp = TempDir::new().expect("temp dir");
        let home = temp.path().join(".codex");
        fs::create_dir_all(home.join("skills/frontend")).expect("skills");
        fs::create_dir_all(home.join("plugins/example")).expect("plugins");
        fs::create_dir_all(home.join("rules")).expect("rules");
        fs::write(home.join("rules/default.rules"), "allow").expect("rule");
        fs::write(home.join("auth.json"), r#"{"token":"never-serialize-me"}"#).expect("auth");
        fs::write(
            home.join("config.toml"),
            r#"
model = "gpt-test"
model_provider = "custom"
model_reasoning_effort = "high"
approval_policy = "on-request"
sandbox_mode = "workspace-write"
web_search = "live"
api_key = "also-never-serialize-me"

[model_providers.custom]
name = "Example"
base_url = "https://user:password@api.example.test/v1"

[mcp_servers.docs]
command = "docs-server"

[features]
fast_mode = true
legacy = false
"#,
        )
        .expect("config");

        let report = discover(&DiscoveryOptions::for_test(temp.path().to_path_buf()));
        assert_eq!(report.homes.len(), 1);
        let found = &report.homes[0];
        assert_eq!(found.endpoint_host.as_deref(), Some("api.example.test"));
        assert_eq!(found.skill_count, 1);
        assert_eq!(found.plugin_count, 1);
        assert_eq!(found.mcp_server_count, 1);
        assert_eq!(found.enabled_feature_count, 1);
        assert_eq!(found.rule_count, 1);
        let output = serde_json::to_string(&report).expect("json");
        assert!(!output.contains("never-serialize-me"));
        assert!(!output.contains("also-never-serialize-me"));
        assert!(!output.contains("password"));
        assert!(report.safety.secrets_redacted);
        assert!(!report.safety.reads_auth_contents);
        assert!(report.safety.includes_local_paths);
    }

    #[test]
    fn malformed_config_is_reported_without_dropping_the_home() {
        let temp = TempDir::new().expect("temp dir");
        let home = temp.path().join(".codex");
        fs::create_dir_all(&home).expect("home");
        fs::write(home.join("config.toml"), "not valid = [").expect("config");

        let report = discover(&DiscoveryOptions::for_test(temp.path().to_path_buf()));
        assert_eq!(report.homes.len(), 1);
        assert!(!report.homes[0].config_valid);
        assert_eq!(report.warnings.len(), 1);
        assert_eq!(report.warnings[0].code, "invalid_config");
    }

    #[test]
    fn doctor_requires_at_least_one_fully_configured_home() {
        let temp = TempDir::new().expect("temp dir");
        let home = temp.path().join(".codex");
        fs::create_dir_all(&home).expect("home");
        fs::write(home.join("config.toml"), "model = \"gpt-test\"").expect("config");

        let result = doctor(discover(&DiscoveryOptions::for_test(
            temp.path().to_path_buf(),
        )));
        assert!(!result.ok);
        assert_eq!(result.home_count, 1);
        assert_eq!(result.healthy_home_count, 0);
        assert_eq!(result.checks[0].issues, ["authentication file is missing"]);
    }

    #[test]
    fn warns_when_an_explicit_home_is_missing() {
        let temp = TempDir::new().expect("temp dir");
        let mut options = DiscoveryOptions::for_test(temp.path().to_path_buf());
        options.additional_homes.push(temp.path().join("missing"));

        let report = discover(&options);
        assert!(report.homes.is_empty());
        assert_eq!(report.warnings.len(), 1);
        assert_eq!(report.warnings[0].code, "home_not_found");
    }
}
