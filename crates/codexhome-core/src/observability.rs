use crate::{user_home, SafetySummary};
use anyhow::{bail, Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub const OBSERVABILITY_EVENT_SCHEMA_VERSION: &str = "codexhome.observability-event.v1";
pub const OBSERVABILITY_SUMMARY_SCHEMA_VERSION: &str = "codexhome.observability-summary.v1";
pub const OBSERVABILITY_EXPORT_SCHEMA_VERSION: &str = "codexhome.observability-export.v1";
pub const OBSERVABILITY_VERIFY_SCHEMA_VERSION: &str = "codexhome.observability-verify.v1";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ObservabilityEventType {
    TaskCreated,
    RunStarted,
    AttemptStarted,
    AttemptCompleted,
    AttemptFailed,
    ThreadLinked,
    ToolCallCompleted,
    ArtifactCreated,
    VerificationCompleted,
    RunCompleted,
    RunFailed,
    QuotaSnapshot,
    RateLimited,
    AuthInvalid,
    HomeHealth,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ObservabilityStatus {
    Started,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
    Retried,
    Degraded,
    Healthy,
    Unhealthy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TraceContext {
    pub task_id: Option<String>,
    pub run_id: Option<String>,
    pub attempt_id: Option<String>,
    pub thread_id: Option<String>,
    pub parent_thread_id: Option<String>,
    pub fork_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutionIdentity {
    pub home_id: Option<String>,
    pub home_alias: Option<String>,
    pub account_id: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub agent_role: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsageDelta {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cached_input_tokens: u64,
    #[serde(default)]
    pub cache_hits: u64,
    #[serde(default)]
    pub cache_misses: u64,
    #[serde(default)]
    pub duration_ms: u64,
    #[serde(default)]
    pub estimated_cost_microusd: u64,
    #[serde(default)]
    pub retries: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FailureInfo {
    pub code: String,
    pub phase: Option<String>,
    pub reason: String,
    #[serde(default)]
    pub retryable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HealthSnapshot {
    pub service_reachable: bool,
    pub auth_valid: Option<bool>,
    pub quota_remaining_basis_points: Option<u16>,
    pub rate_limit_reset_at_ms: Option<u64>,
    pub detail_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EventSafety {
    pub secrets_redacted: bool,
    pub includes_secret_values: bool,
    pub reads_auth_contents: bool,
    pub includes_local_paths: bool,
}

impl Default for EventSafety {
    fn default() -> Self {
        Self {
            secrets_redacted: true,
            includes_secret_values: false,
            reads_auth_contents: false,
            includes_local_paths: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObservabilityEvent {
    pub schema_version: String,
    pub event_id: String,
    pub timestamp_ms: u64,
    pub event_type: ObservabilityEventType,
    pub status: ObservabilityStatus,
    pub trace: TraceContext,
    pub identity: ExecutionIdentity,
    #[serde(default)]
    pub usage: UsageDelta,
    pub parent_event_id: Option<String>,
    pub tool_name: Option<String>,
    pub artifact_kind: Option<String>,
    pub verification_kind: Option<String>,
    pub failure: Option<FailureInfo>,
    pub health: Option<HealthSnapshot>,
    pub safety: EventSafety,
}

impl ObservabilityEvent {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != OBSERVABILITY_EVENT_SCHEMA_VERSION {
            bail!(
                "event {} uses unsupported schemaVersion '{}'",
                self.event_id,
                self.schema_version
            );
        }
        validate_identifier("eventId", &self.event_id)?;
        for (name, value) in [
            ("taskId", self.trace.task_id.as_deref()),
            ("runId", self.trace.run_id.as_deref()),
            ("attemptId", self.trace.attempt_id.as_deref()),
            ("threadId", self.trace.thread_id.as_deref()),
            ("parentThreadId", self.trace.parent_thread_id.as_deref()),
            ("forkId", self.trace.fork_id.as_deref()),
            ("parentEventId", self.parent_event_id.as_deref()),
            ("homeId", self.identity.home_id.as_deref()),
            ("accountId", self.identity.account_id.as_deref()),
        ] {
            if let Some(value) = value {
                validate_identifier(name, value)?;
            }
        }
        for (name, value) in [
            ("homeAlias", self.identity.home_alias.as_deref()),
            ("provider", self.identity.provider.as_deref()),
            ("model", self.identity.model.as_deref()),
            ("agentRole", self.identity.agent_role.as_deref()),
            ("toolName", self.tool_name.as_deref()),
            ("artifactKind", self.artifact_kind.as_deref()),
            ("verificationKind", self.verification_kind.as_deref()),
        ] {
            if let Some(value) = value {
                validate_label(name, value)?;
            }
        }
        if self.usage.cached_input_tokens > self.usage.input_tokens {
            bail!(
                "event {} has cachedInputTokens greater than inputTokens",
                self.event_id
            );
        }
        if !self.safety.secrets_redacted
            || self.safety.includes_secret_values
            || self.safety.reads_auth_contents
        {
            bail!(
                "event {} violates the public-safe observability contract",
                self.event_id
            );
        }
        if let Some(failure) = &self.failure {
            validate_label("failure.code", &failure.code)?;
            if let Some(phase) = &failure.phase {
                validate_label("failure.phase", phase)?;
            }
            let reason = failure.reason.trim();
            if reason.is_empty() || reason.len() > 2048 {
                bail!("event {} has an invalid failure reason", self.event_id);
            }
            if contains_obvious_secret(reason) {
                bail!(
                    "event {} failure reason appears to contain a secret",
                    self.event_id
                );
            }
        }
        validate_event_shape(self)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservabilityAppendReport {
    pub ok: bool,
    pub schema_version: &'static str,
    pub store_path: String,
    pub appended: usize,
    pub total_events: usize,
    pub first_event_id: Option<String>,
    pub last_event_id: Option<String>,
    pub safety: SafetySummary,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservabilityVerifyReport {
    pub ok: bool,
    pub schema_version: &'static str,
    pub store_path: String,
    pub event_count: usize,
    pub task_count: usize,
    pub run_count: usize,
    pub attempt_count: usize,
    pub thread_count: usize,
    pub safety: SafetySummary,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservabilitySummary {
    pub ok: bool,
    pub schema_version: &'static str,
    pub store_path: String,
    pub as_of_timestamp_ms: Option<u64>,
    pub event_count: usize,
    pub totals: ObservabilityTotals,
    pub by_home: Vec<ObservabilityGroup>,
    pub by_account: Vec<ObservabilityGroup>,
    pub by_model: Vec<ObservabilityGroup>,
    pub by_thread: Vec<ObservabilityGroup>,
    pub latest_home_health: Vec<LatestHomeHealth>,
    pub safety: SafetySummary,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ObservabilityTotals {
    pub tasks: usize,
    pub runs: usize,
    pub attempts: usize,
    pub threads: usize,
    pub tool_calls: u64,
    pub artifacts: u64,
    pub verifications: u64,
    pub completed_attempts: u64,
    pub failed_attempts: u64,
    pub failure_rate_basis_points: u16,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub cached_input_tokens: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub cache_hit_rate_basis_points: u16,
    pub duration_ms: u64,
    pub estimated_cost_microusd: u64,
    pub retries: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservabilityGroup {
    pub key: String,
    pub totals: ObservabilityTotals,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LatestHomeHealth {
    pub home_id: String,
    pub timestamp_ms: u64,
    pub status: ObservabilityStatus,
    pub snapshot: HealthSnapshot,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservabilityExport {
    pub ok: bool,
    pub schema_version: &'static str,
    pub store_path: String,
    pub events: Vec<ObservabilityEvent>,
    pub safety: SafetySummary,
}

#[derive(Debug, Clone, Default)]
pub struct ObservabilityFilter {
    pub home: Option<String>,
    pub account: Option<String>,
    pub model: Option<String>,
    pub thread: Option<String>,
    pub run: Option<String>,
}

impl ObservabilityFilter {
    pub fn filter<'a>(
        &self,
        events: impl IntoIterator<Item = &'a ObservabilityEvent>,
    ) -> Vec<ObservabilityEvent> {
        events
            .into_iter()
            .filter(|event| self.matches_all(event))
            .cloned()
            .collect()
    }

    fn matches_all(&self, event: &ObservabilityEvent) -> bool {
        let home_matches = self.home.as_ref().is_none_or(|expected| {
            event.identity.home_id.as_deref() == Some(expected)
                || event.identity.home_alias.as_deref() == Some(expected)
        });
        home_matches
            && matches_optional(&self.account, event.identity.account_id.as_deref())
            && matches_optional(&self.model, event.identity.model.as_deref())
            && matches_optional(&self.thread, event.trace.thread_id.as_deref())
            && matches_optional(&self.run, event.trace.run_id.as_deref())
    }
}

#[derive(Debug, Clone)]
pub struct ObservabilityStore {
    path: PathBuf,
}

impl ObservabilityStore {
    pub fn from_environment(override_path: Option<PathBuf>) -> Result<Self> {
        let home = user_home().context("cannot discover the current user home directory")?;
        let path = override_path
            .or_else(|| env::var_os("CODEXHOME_OBSERVABILITY_STORE").map(PathBuf::from))
            .unwrap_or_else(|| home.join(".codexhome/observability/events.jsonl"));
        Ok(Self::new(path))
    }

    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<Vec<ObservabilityEvent>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let file = File::open(&self.path)
            .with_context(|| format!("failed to open {}", self.path.display()))?;
        load_events(file, &self.path)
    }

    pub fn append(&self, incoming: &[ObservabilityEvent]) -> Result<ObservabilityAppendReport> {
        if incoming.is_empty() {
            bail!("at least one observability event is required");
        }
        let parent = self
            .path
            .parent()
            .context("observability store path has no parent directory")?;
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
        secure_directory(parent)?;
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(&self.path)
            .with_context(|| format!("failed to open {}", self.path.display()))?;
        secure_file(&file)?;
        file.lock_exclusive()
            .with_context(|| format!("failed to lock {}", self.path.display()))?;
        file.seek(SeekFrom::Start(0))?;
        let existing = load_events(&file, &self.path)?;
        let mut combined = existing.clone();
        combined.extend_from_slice(incoming);
        verify_events(&combined)?;
        file.seek(SeekFrom::End(0))?;
        for event in incoming {
            serde_json::to_writer(&mut file, event)?;
            file.write_all(b"\n")?;
        }
        file.sync_data()
            .with_context(|| format!("failed to sync {}", self.path.display()))?;
        FileExt::unlock(&file)?;
        Ok(ObservabilityAppendReport {
            ok: true,
            schema_version: OBSERVABILITY_EVENT_SCHEMA_VERSION,
            store_path: self.path.to_string_lossy().into_owned(),
            appended: incoming.len(),
            total_events: combined.len(),
            first_event_id: incoming.first().map(|event| event.event_id.clone()),
            last_event_id: incoming.last().map(|event| event.event_id.clone()),
            safety: observability_safety(),
        })
    }

    pub fn verify(&self) -> Result<ObservabilityVerifyReport> {
        let events = self.load()?;
        let indexes = verify_events(&events)?;
        Ok(ObservabilityVerifyReport {
            ok: true,
            schema_version: OBSERVABILITY_VERIFY_SCHEMA_VERSION,
            store_path: self.path.to_string_lossy().into_owned(),
            event_count: events.len(),
            task_count: indexes.tasks.len(),
            run_count: indexes.runs.len(),
            attempt_count: indexes.attempts.len(),
            thread_count: indexes.threads.len(),
            safety: observability_safety(),
        })
    }

    pub fn summary(&self, filter: &ObservabilityFilter) -> Result<ObservabilitySummary> {
        let events = self.load()?;
        verify_events(&events)?;
        let filtered = filter.filter(&events);
        summarize(&filtered, &self.path)
    }

    pub fn export(&self, filter: &ObservabilityFilter) -> Result<ObservabilityExport> {
        let events = self.load()?;
        verify_events(&events)?;
        Ok(ObservabilityExport {
            ok: true,
            schema_version: OBSERVABILITY_EXPORT_SCHEMA_VERSION,
            store_path: self.path.to_string_lossy().into_owned(),
            events: filter.filter(&events),
            safety: observability_safety(),
        })
    }
}

pub fn parse_observability_events(input: &str) -> Result<Vec<ObservabilityEvent>> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        bail!("observability input is empty");
    }
    if let Ok(event) = serde_json::from_str::<ObservabilityEvent>(trimmed) {
        return Ok(vec![event]);
    }
    if let Ok(events) = serde_json::from_str::<Vec<ObservabilityEvent>>(trimmed) {
        return Ok(events);
    }
    trimmed
        .lines()
        .enumerate()
        .map(|(index, line)| {
            serde_json::from_str::<ObservabilityEvent>(line)
                .with_context(|| format!("invalid observability JSONL at line {}", index + 1))
        })
        .collect()
}

pub fn observability_events_csv(events: &[ObservabilityEvent]) -> String {
    let mut output = String::from(
        "schema_version,event_id,timestamp_ms,event_type,status,task_id,run_id,attempt_id,thread_id,home_id,home_alias,account_id,provider,model,input_tokens,output_tokens,cached_input_tokens,cache_hits,cache_misses,duration_ms,estimated_cost_microusd,retries,failure_code,failure_phase,failure_reason\n",
    );
    for event in events {
        let fields = [
            event.schema_version.clone(),
            event.event_id.clone(),
            event.timestamp_ms.to_string(),
            json_name(&event.event_type),
            json_name(&event.status),
            option(&event.trace.task_id),
            option(&event.trace.run_id),
            option(&event.trace.attempt_id),
            option(&event.trace.thread_id),
            option(&event.identity.home_id),
            option(&event.identity.home_alias),
            option(&event.identity.account_id),
            option(&event.identity.provider),
            option(&event.identity.model),
            event.usage.input_tokens.to_string(),
            event.usage.output_tokens.to_string(),
            event.usage.cached_input_tokens.to_string(),
            event.usage.cache_hits.to_string(),
            event.usage.cache_misses.to_string(),
            event.usage.duration_ms.to_string(),
            event.usage.estimated_cost_microusd.to_string(),
            event.usage.retries.to_string(),
            event
                .failure
                .as_ref()
                .map(|failure| failure.code.clone())
                .unwrap_or_default(),
            event
                .failure
                .as_ref()
                .and_then(|failure| failure.phase.clone())
                .unwrap_or_default(),
            event
                .failure
                .as_ref()
                .map(|failure| failure.reason.clone())
                .unwrap_or_default(),
        ];
        output.push_str(
            &fields
                .iter()
                .map(|value| csv_cell(value))
                .collect::<Vec<_>>()
                .join(","),
        );
        output.push('\n');
    }
    output
}

fn summarize(events: &[ObservabilityEvent], path: &Path) -> Result<ObservabilitySummary> {
    let mut overall = Accumulator::default();
    let mut by_home = BTreeMap::<String, Accumulator>::new();
    let mut by_account = BTreeMap::<String, Accumulator>::new();
    let mut by_model = BTreeMap::<String, Accumulator>::new();
    let mut by_thread = BTreeMap::<String, Accumulator>::new();
    let mut latest_health = BTreeMap::<String, LatestHomeHealth>::new();
    for event in events {
        overall.add(event)?;
        add_group(&mut by_home, event.identity.home_id.as_deref(), event)?;
        add_group(&mut by_account, event.identity.account_id.as_deref(), event)?;
        add_group(&mut by_model, event.identity.model.as_deref(), event)?;
        add_group(&mut by_thread, event.trace.thread_id.as_deref(), event)?;
        if let (Some(home_id), Some(snapshot)) =
            (event.identity.home_id.as_ref(), event.health.as_ref())
        {
            let replace = latest_health
                .get(home_id)
                .is_none_or(|current| event.timestamp_ms >= current.timestamp_ms);
            if replace {
                latest_health.insert(
                    home_id.clone(),
                    LatestHomeHealth {
                        home_id: home_id.clone(),
                        timestamp_ms: event.timestamp_ms,
                        status: event.status,
                        snapshot: snapshot.clone(),
                    },
                );
            }
        }
    }
    Ok(ObservabilitySummary {
        ok: true,
        schema_version: OBSERVABILITY_SUMMARY_SCHEMA_VERSION,
        store_path: path.to_string_lossy().into_owned(),
        as_of_timestamp_ms: events.iter().map(|event| event.timestamp_ms).max(),
        event_count: events.len(),
        totals: overall.finish(),
        by_home: finish_groups(by_home),
        by_account: finish_groups(by_account),
        by_model: finish_groups(by_model),
        by_thread: finish_groups(by_thread),
        latest_home_health: latest_health.into_values().collect(),
        safety: observability_safety(),
    })
}

#[derive(Default)]
struct Accumulator {
    tasks: BTreeSet<String>,
    runs: BTreeSet<String>,
    attempts: BTreeSet<String>,
    threads: BTreeSet<String>,
    totals: ObservabilityTotals,
}

impl Accumulator {
    fn add(&mut self, event: &ObservabilityEvent) -> Result<()> {
        if let Some(value) = &event.trace.task_id {
            self.tasks.insert(value.clone());
        }
        if let Some(value) = &event.trace.run_id {
            self.runs.insert(value.clone());
        }
        if let Some(value) = &event.trace.attempt_id {
            self.attempts.insert(value.clone());
        }
        if let Some(value) = &event.trace.thread_id {
            self.threads.insert(value.clone());
        }
        match event.event_type {
            ObservabilityEventType::ToolCallCompleted => {
                checked_add(&mut self.totals.tool_calls, 1, "tool call count")?
            }
            ObservabilityEventType::ArtifactCreated => {
                checked_add(&mut self.totals.artifacts, 1, "artifact count")?
            }
            ObservabilityEventType::VerificationCompleted => {
                checked_add(&mut self.totals.verifications, 1, "verification count")?
            }
            ObservabilityEventType::AttemptCompleted => {
                checked_add(
                    &mut self.totals.completed_attempts,
                    1,
                    "completed attempt count",
                )?;
            }
            ObservabilityEventType::AttemptFailed => {
                checked_add(&mut self.totals.failed_attempts, 1, "failed attempt count")?;
            }
            _ => {}
        }
        for (target, value, label) in [
            (
                &mut self.totals.input_tokens,
                event.usage.input_tokens,
                "input tokens",
            ),
            (
                &mut self.totals.output_tokens,
                event.usage.output_tokens,
                "output tokens",
            ),
            (
                &mut self.totals.cached_input_tokens,
                event.usage.cached_input_tokens,
                "cached input tokens",
            ),
            (
                &mut self.totals.cache_hits,
                event.usage.cache_hits,
                "cache hits",
            ),
            (
                &mut self.totals.cache_misses,
                event.usage.cache_misses,
                "cache misses",
            ),
            (
                &mut self.totals.duration_ms,
                event.usage.duration_ms,
                "duration",
            ),
            (
                &mut self.totals.estimated_cost_microusd,
                event.usage.estimated_cost_microusd,
                "estimated cost",
            ),
            (&mut self.totals.retries, event.usage.retries, "retry count"),
        ] {
            checked_add(target, value, label)?;
        }
        Ok(())
    }

    fn finish(mut self) -> ObservabilityTotals {
        self.totals.tasks = self.tasks.len();
        self.totals.runs = self.runs.len();
        self.totals.attempts = self.attempts.len();
        self.totals.threads = self.threads.len();
        self.totals.total_tokens = self
            .totals
            .input_tokens
            .saturating_add(self.totals.output_tokens);
        let terminal_attempts = self
            .totals
            .completed_attempts
            .saturating_add(self.totals.failed_attempts);
        self.totals.failure_rate_basis_points =
            ratio_basis_points(self.totals.failed_attempts, terminal_attempts);
        self.totals.cache_hit_rate_basis_points = ratio_basis_points(
            self.totals.cache_hits,
            self.totals
                .cache_hits
                .saturating_add(self.totals.cache_misses),
        );
        self.totals
    }
}

#[derive(Debug, Default)]
struct TraceIndexes {
    events: BTreeSet<String>,
    tasks: BTreeSet<String>,
    runs: BTreeMap<String, String>,
    attempts: BTreeMap<String, (String, String)>,
    threads: BTreeMap<String, (String, String)>,
    terminal_attempts: BTreeSet<String>,
    terminal_runs: BTreeSet<String>,
    last_timestamp_by_run: BTreeMap<String, u64>,
}

fn verify_events(events: &[ObservabilityEvent]) -> Result<TraceIndexes> {
    let mut indexes = TraceIndexes::default();
    for event in events {
        event.validate()?;
        if !indexes.events.insert(event.event_id.clone()) {
            bail!("duplicate observability eventId '{}'", event.event_id);
        }
        if let Some(parent) = &event.parent_event_id {
            if !indexes.events.contains(parent) {
                bail!(
                    "event {} references missing or future parentEventId '{}'",
                    event.event_id,
                    parent
                );
            }
        }
        if let Some(run_id) = &event.trace.run_id {
            if let Some(previous) = indexes.last_timestamp_by_run.get(run_id) {
                if event.timestamp_ms < *previous {
                    bail!(
                        "event {} timestamp moves backwards within run '{}'",
                        event.event_id,
                        run_id
                    );
                }
            }
            indexes
                .last_timestamp_by_run
                .insert(run_id.clone(), event.timestamp_ms);
        }
        verify_links(event, &mut indexes)?;
    }
    Ok(indexes)
}

fn verify_links(event: &ObservabilityEvent, indexes: &mut TraceIndexes) -> Result<()> {
    let task_id = event.trace.task_id.as_deref();
    let run_id = event.trace.run_id.as_deref();
    let attempt_id = event.trace.attempt_id.as_deref();
    let thread_id = event.trace.thread_id.as_deref();
    match event.event_type {
        ObservabilityEventType::TaskCreated => {
            let task = required("taskId", task_id)?;
            if !indexes.tasks.insert(task.to_owned()) {
                bail!("taskId '{}' is created more than once", task);
            }
        }
        ObservabilityEventType::RunStarted => {
            let task = required("taskId", task_id)?;
            ensure_exists("taskId", task, &indexes.tasks, &event.event_id)?;
            let run = required("runId", run_id)?;
            if indexes
                .runs
                .insert(run.to_owned(), task.to_owned())
                .is_some()
            {
                bail!("runId '{}' is started more than once", run);
            }
        }
        ObservabilityEventType::AttemptStarted => {
            let (task, run) = verify_run_link(event, &indexes.runs)?;
            let attempt = required("attemptId", attempt_id)?;
            if indexes
                .attempts
                .insert(attempt.to_owned(), (task.to_owned(), run.to_owned()))
                .is_some()
            {
                bail!("attemptId '{}' is started more than once", attempt);
            }
        }
        ObservabilityEventType::ThreadLinked => {
            let (task, run) = verify_run_link(event, &indexes.runs)?;
            let thread = required("threadId", thread_id)?;
            if indexes
                .threads
                .insert(thread.to_owned(), (task.to_owned(), run.to_owned()))
                .is_some()
            {
                bail!("threadId '{}' is linked more than once", thread);
            }
            if let Some(parent) = &event.trace.parent_thread_id {
                if !indexes.threads.contains_key(parent) {
                    bail!(
                        "event {} references unknown parentThreadId '{}'",
                        event.event_id,
                        parent
                    );
                }
            }
        }
        ObservabilityEventType::AttemptCompleted
        | ObservabilityEventType::AttemptFailed
        | ObservabilityEventType::ToolCallCompleted
        | ObservabilityEventType::ArtifactCreated
        | ObservabilityEventType::VerificationCompleted => {
            let (task, run) = verify_run_link(event, &indexes.runs)?;
            let attempt = required("attemptId", attempt_id)?;
            let expected = indexes.attempts.get(attempt).with_context(|| {
                format!(
                    "event {} references unknown attemptId '{}'",
                    event.event_id, attempt
                )
            })?;
            if expected != &(task.to_owned(), run.to_owned()) {
                bail!(
                    "event {} attemptId '{}' belongs to another task/run",
                    event.event_id,
                    attempt
                );
            }
            verify_optional_thread(event, &indexes.threads, task, run)?;
            if matches!(
                event.event_type,
                ObservabilityEventType::AttemptCompleted | ObservabilityEventType::AttemptFailed
            ) && !indexes.terminal_attempts.insert(attempt.to_owned())
            {
                bail!("attemptId '{}' has more than one terminal event", attempt);
            }
        }
        ObservabilityEventType::RunCompleted | ObservabilityEventType::RunFailed => {
            let (_, run) = verify_run_link(event, &indexes.runs)?;
            if !indexes.terminal_runs.insert(run.to_owned()) {
                bail!("runId '{}' has more than one terminal event", run);
            }
        }
        ObservabilityEventType::QuotaSnapshot
        | ObservabilityEventType::RateLimited
        | ObservabilityEventType::AuthInvalid
        | ObservabilityEventType::HomeHealth => {}
    }
    Ok(())
}

fn verify_run_link<'a>(
    event: &'a ObservabilityEvent,
    runs: &'a BTreeMap<String, String>,
) -> Result<(&'a str, &'a str)> {
    let task = required("taskId", event.trace.task_id.as_deref())?;
    let run = required("runId", event.trace.run_id.as_deref())?;
    let owner = runs.get(run).with_context(|| {
        format!(
            "event {} references unknown runId '{}'",
            event.event_id, run
        )
    })?;
    if owner != task {
        bail!(
            "event {} runId '{}' belongs to another task",
            event.event_id,
            run
        );
    }
    Ok((task, run))
}

fn verify_optional_thread(
    event: &ObservabilityEvent,
    threads: &BTreeMap<String, (String, String)>,
    task: &str,
    run: &str,
) -> Result<()> {
    let Some(thread) = &event.trace.thread_id else {
        return Ok(());
    };
    let expected = threads.get(thread).with_context(|| {
        format!(
            "event {} references unknown threadId '{}'",
            event.event_id, thread
        )
    })?;
    if expected != &(task.to_owned(), run.to_owned()) {
        bail!(
            "event {} threadId '{}' belongs to another task/run",
            event.event_id,
            thread
        );
    }
    Ok(())
}

fn validate_event_shape(event: &ObservabilityEvent) -> Result<()> {
    use ObservabilityEventType as Type;
    if matches!(
        event.event_type,
        Type::AttemptStarted
            | Type::AttemptCompleted
            | Type::AttemptFailed
            | Type::ToolCallCompleted
            | Type::ArtifactCreated
            | Type::VerificationCompleted
    ) {
        required("homeId", event.identity.home_id.as_deref())?;
        required("model", event.identity.model.as_deref())?;
    }
    if event.event_type == Type::ToolCallCompleted {
        required("toolName", event.tool_name.as_deref())?;
    }
    if event.event_type == Type::ArtifactCreated {
        required("artifactKind", event.artifact_kind.as_deref())?;
    }
    if event.event_type == Type::VerificationCompleted {
        required("verificationKind", event.verification_kind.as_deref())?;
    }
    if matches!(
        event.event_type,
        Type::QuotaSnapshot | Type::RateLimited | Type::AuthInvalid | Type::HomeHealth
    ) {
        required("homeId", event.identity.home_id.as_deref())?;
        if event.health.is_none() {
            bail!("event {} requires a health snapshot", event.event_id);
        }
    }
    if matches!(
        event.event_type,
        Type::AttemptFailed | Type::RunFailed | Type::RateLimited | Type::AuthInvalid
    ) && event.failure.is_none()
    {
        bail!("event {} requires failure details", event.event_id);
    }
    if matches!(
        event.event_type,
        Type::AttemptCompleted | Type::RunCompleted
    ) && event.status != ObservabilityStatus::Succeeded
    {
        bail!("event {} must use status 'succeeded'", event.event_id);
    }
    if matches!(event.event_type, Type::AttemptFailed | Type::RunFailed)
        && !matches!(
            event.status,
            ObservabilityStatus::Failed
                | ObservabilityStatus::Cancelled
                | ObservabilityStatus::TimedOut
        )
    {
        bail!(
            "event {} must use a terminal failure status",
            event.event_id
        );
    }
    if event.event_type == Type::TaskCreated
        && (event.trace.run_id.is_some()
            || event.trace.attempt_id.is_some()
            || event.trace.thread_id.is_some())
    {
        bail!(
            "task_created event {} cannot pre-bind run IDs",
            event.event_id
        );
    }
    Ok(())
}

fn load_events(file: impl std::io::Read, path: &Path) -> Result<Vec<ObservabilityEvent>> {
    BufReader::new(file)
        .lines()
        .enumerate()
        .filter_map(|(index, line)| match line {
            Ok(line) if line.trim().is_empty() => None,
            other => Some((index, other)),
        })
        .map(|(index, line)| {
            let line = line
                .with_context(|| format!("failed to read {} line {}", path.display(), index + 1))?;
            serde_json::from_str::<ObservabilityEvent>(&line).with_context(|| {
                format!(
                    "invalid observability event in {} at line {}",
                    path.display(),
                    index + 1
                )
            })
        })
        .collect()
}

fn add_group(
    groups: &mut BTreeMap<String, Accumulator>,
    key: Option<&str>,
    event: &ObservabilityEvent,
) -> Result<()> {
    if let Some(key) = key {
        groups.entry(key.to_owned()).or_default().add(event)?;
    }
    Ok(())
}

fn finish_groups(groups: BTreeMap<String, Accumulator>) -> Vec<ObservabilityGroup> {
    groups
        .into_iter()
        .map(|(key, accumulator)| ObservabilityGroup {
            key,
            totals: accumulator.finish(),
        })
        .collect()
}

fn required<'a>(name: &str, value: Option<&'a str>) -> Result<&'a str> {
    value
        .filter(|value| !value.trim().is_empty())
        .with_context(|| format!("{name} is required for this observability event"))
}

fn ensure_exists(name: &str, value: &str, values: &BTreeSet<String>, event_id: &str) -> Result<()> {
    if !values.contains(value) {
        bail!("event {event_id} references unknown {name} '{value}'");
    }
    Ok(())
}

fn validate_identifier(name: &str, value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 160
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._:-/@".contains(character))
    {
        bail!("{name} contains unsupported characters");
    }
    Ok(())
}

fn validate_label(name: &str, value: &str) -> Result<()> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.len() > 256
        || trimmed
            .chars()
            .any(|character| matches!(character, '\r' | '\n'))
    {
        bail!("{name} is empty or too long");
    }
    if contains_obvious_secret(trimmed) {
        bail!("{name} appears to contain a secret");
    }
    Ok(())
}

fn contains_obvious_secret(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.contains("authorization: bearer")
        || lower.contains("api_key=")
        || lower.contains("apikey=")
        || lower.contains("access_token=")
        || lower.contains("refresh_token=")
        || lower.split_whitespace().any(|part| {
            (part.starts_with("sk-") || part.starts_with("ghp_") || part.starts_with("github_pat_"))
                && part.len() > 16
        })
}

fn matches_optional(expected: &Option<String>, actual: Option<&str>) -> bool {
    expected.as_ref().is_none_or(|value| actual == Some(value))
}

fn checked_add(target: &mut u64, value: u64, label: &str) -> Result<()> {
    *target = target
        .checked_add(value)
        .with_context(|| format!("{label} overflow"))?;
    Ok(())
}

fn ratio_basis_points(numerator: u64, denominator: u64) -> u16 {
    if denominator == 0 {
        return 0;
    }
    ((u128::from(numerator) * 10_000 / u128::from(denominator)).min(10_000)) as u16
}

fn option(value: &Option<String>) -> String {
    value.clone().unwrap_or_default()
}

fn json_name(value: &impl Serialize) -> String {
    serde_json::to_string(value)
        .unwrap_or_else(|_| "\"unknown\"".to_owned())
        .trim_matches('"')
        .to_owned()
}

fn csv_cell(value: &str) -> String {
    if value
        .chars()
        .any(|character| matches!(character, ',' | '"' | '\r' | '\n'))
    {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

fn observability_safety() -> SafetySummary {
    SafetySummary {
        secrets_redacted: true,
        includes_secrets: false,
        reads_auth_contents: false,
        includes_local_paths: true,
    }
}

#[cfg(unix)]
fn secure_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("failed to secure {}", path.display()))
}

#[cfg(not(unix))]
fn secure_directory(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn secure_file(file: &File) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .context("failed to secure observability store")
}

#[cfg(not(unix))]
fn secure_file(_file: &File) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn event(
        id: &str,
        timestamp_ms: u64,
        event_type: ObservabilityEventType,
    ) -> ObservabilityEvent {
        ObservabilityEvent {
            schema_version: OBSERVABILITY_EVENT_SCHEMA_VERSION.to_owned(),
            event_id: id.to_owned(),
            timestamp_ms,
            event_type,
            status: ObservabilityStatus::Running,
            trace: TraceContext {
                task_id: Some("task-1".to_owned()),
                run_id: None,
                attempt_id: None,
                thread_id: None,
                parent_thread_id: None,
                fork_id: None,
            },
            identity: ExecutionIdentity::default(),
            usage: UsageDelta::default(),
            parent_event_id: None,
            tool_name: None,
            artifact_kind: None,
            verification_kind: None,
            failure: None,
            health: None,
            safety: EventSafety::default(),
        }
    }

    fn trace_fixture() -> Vec<ObservabilityEvent> {
        let task = event("evt-1", 1, ObservabilityEventType::TaskCreated);
        let mut run = event("evt-2", 2, ObservabilityEventType::RunStarted);
        run.trace.run_id = Some("run-1".to_owned());
        let mut attempt = event("evt-3", 3, ObservabilityEventType::AttemptStarted);
        attempt.trace.run_id = Some("run-1".to_owned());
        attempt.trace.attempt_id = Some("attempt-1".to_owned());
        attempt.identity.home_id = Some("home-1".to_owned());
        attempt.identity.account_id = Some("account-1".to_owned());
        attempt.identity.model = Some("gpt-test".to_owned());
        let mut thread = event("evt-4", 4, ObservabilityEventType::ThreadLinked);
        thread.trace.run_id = Some("run-1".to_owned());
        thread.trace.thread_id = Some("thread-1".to_owned());
        let mut complete = event("evt-5", 5, ObservabilityEventType::AttemptCompleted);
        complete.status = ObservabilityStatus::Succeeded;
        complete.trace.run_id = Some("run-1".to_owned());
        complete.trace.attempt_id = Some("attempt-1".to_owned());
        complete.trace.thread_id = Some("thread-1".to_owned());
        complete.identity.home_id = Some("home-1".to_owned());
        complete.identity.account_id = Some("account-1".to_owned());
        complete.identity.model = Some("gpt-test".to_owned());
        complete.usage = UsageDelta {
            input_tokens: 100,
            output_tokens: 40,
            cached_input_tokens: 60,
            cache_hits: 1,
            cache_misses: 0,
            duration_ms: 1500,
            estimated_cost_microusd: 1200,
            retries: 0,
        };
        vec![task, run, attempt, thread, complete]
    }

    #[test]
    fn append_verify_and_summary_preserve_trace_and_cost_facts() {
        let temp = TempDir::new().expect("temp");
        let store = ObservabilityStore::new(temp.path().join("events.jsonl"));
        let events = trace_fixture();
        let report = store.append(&events).expect("append");
        assert_eq!(report.appended, 5);
        assert_eq!(store.verify().expect("verify").attempt_count, 1);
        let summary = store
            .summary(&ObservabilityFilter::default())
            .expect("summary");
        assert_eq!(summary.totals.tasks, 1);
        assert_eq!(summary.totals.runs, 1);
        assert_eq!(summary.totals.attempts, 1);
        assert_eq!(summary.totals.threads, 1);
        assert_eq!(summary.totals.total_tokens, 140);
        assert_eq!(summary.totals.cached_input_tokens, 60);
        assert_eq!(summary.totals.cache_hit_rate_basis_points, 10_000);
        assert_eq!(summary.totals.duration_ms, 1500);
        assert_eq!(summary.totals.estimated_cost_microusd, 1200);
        assert_eq!(summary.by_home[0].key, "home-1");
        assert_eq!(summary.by_account[0].key, "account-1");
        assert_eq!(summary.by_model[0].key, "gpt-test");
        assert_eq!(summary.by_thread[0].key, "thread-1");
    }

    #[test]
    fn rejects_duplicate_ids_broken_links_and_secret_like_failure_text() {
        let mut events = trace_fixture();
        events[1].event_id = "evt-1".to_owned();
        assert!(verify_events(&events)
            .expect_err("duplicate")
            .to_string()
            .contains("duplicate"));

        let mut events = trace_fixture();
        events[2].trace.run_id = Some("missing-run".to_owned());
        assert!(verify_events(&events)
            .expect_err("broken link")
            .to_string()
            .contains("unknown runId"));

        let mut failed = trace_fixture()[4].clone();
        failed.event_type = ObservabilityEventType::AttemptFailed;
        failed.status = ObservabilityStatus::Failed;
        failed.failure = Some(FailureInfo {
            code: "provider_error".to_owned(),
            phase: Some("inference".to_owned()),
            reason: "Authorization: Bearer sk-this-must-not-enter-observability".to_owned(),
            retryable: true,
        });
        assert!(failed
            .validate()
            .expect_err("secret")
            .to_string()
            .contains("secret"));
    }

    #[test]
    fn rejects_missing_type_specific_fields_and_unknown_payloads() {
        let mut tool_event = trace_fixture()[2].clone();
        tool_event.event_id = "evt-tool".to_owned();
        tool_event.event_type = ObservabilityEventType::ToolCallCompleted;
        assert!(tool_event
            .validate()
            .expect_err("tool name is required")
            .to_string()
            .contains("toolName"));

        let unknown_field = r#"{
            "schemaVersion":"codexhome.observability-event.v1",
            "eventId":"evt-1",
            "timestampMs":1,
            "eventType":"task_created",
            "status":"started",
            "trace":{"taskId":"task-1"},
            "identity":{},
            "safety":{
                "secretsRedacted":true,
                "includesSecretValues":false,
                "readsAuthContents":false,
                "includesLocalPaths":false
            },
            "payload":{"prompt":"must not be accepted"}
        }"#;
        assert!(parse_observability_events(unknown_field).is_err());
    }

    #[test]
    fn parses_json_arrays_and_exports_deterministic_csv() {
        let events = trace_fixture();
        let encoded = serde_json::to_string(&events).expect("encode");
        let parsed = parse_observability_events(&encoded).expect("parse");
        assert_eq!(parsed.len(), 5);
        let csv = observability_events_csv(&parsed);
        assert!(csv.starts_with("schema_version,event_id,timestamp_ms"));
        assert!(csv.contains("attempt_completed"));
        assert!(csv.contains("gpt-test,100,40,60"));
    }
}
