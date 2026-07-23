use crate::{router::RouteRequest, user_home, SafetySummary};
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
    WorktreePrepared,
    WorktreeEvidenceRecorded,
    WorktreeReviewRecorded,
    WorktreeConflictChecked,
    AttemptCompleted,
    AttemptFailed,
    ThreadLinked,
    ToolCallCompleted,
    ArtifactCreated,
    VerificationCompleted,
    RouteDecided,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskDescriptor {
    pub label: String,
    pub kind: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunBudget {
    pub max_total_tokens: Option<u64>,
    pub max_duration_ms: Option<u64>,
    pub max_cost_microusd: Option<u64>,
    pub max_attempts: Option<u32>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AttemptTransitionKind {
    Initial,
    Retry,
    Migration,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttemptTransition {
    pub kind: AttemptTransitionKind,
    pub from_attempt_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EventDetails {
    pub task: Option<TaskDescriptor>,
    pub budget: Option<RunBudget>,
    pub route_reason: Option<String>,
    pub route_decision_id: Option<String>,
    pub routing: Option<RouteDecisionDetails>,
    pub transition: Option<AttemptTransition>,
    pub worktree: Option<WorktreePreparedDetails>,
    pub worktree_evidence: Option<WorktreeEvidenceDetails>,
    pub worktree_review: Option<WorktreeReviewDetails>,
    pub worktree_conflict: Option<WorktreeConflictDetails>,
    pub artifact_id: Option<String>,
    pub verification_id: Option<String>,
    pub target_artifact_id: Option<String>,
    #[serde(default)]
    pub final_artifact_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorktreePreparedDetails {
    pub repository_root: String,
    pub worktree_path: String,
    pub branch: String,
    pub base_ref: String,
    pub base_commit: String,
    pub head_commit: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorktreeEvidenceDetails {
    pub evidence_id: String,
    pub base_commit: String,
    pub head_commit: String,
    pub commit_count: u32,
    pub diff_sha256: String,
    pub patch_path: String,
    pub patch_bytes: u64,
    pub changed_files: u32,
    pub insertions: u64,
    pub deletions: u64,
    pub worktree_clean: bool,
    pub tests_succeeded: bool,
    pub test_label: String,
    pub test_exit_code: i32,
    pub test_duration_ms: u64,
    pub test_output_sha256: String,
    pub test_log_path: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeReviewDecision {
    Approved,
    ChangesRequested,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorktreeReviewDetails {
    pub review_id: String,
    pub evidence_id: String,
    pub decision: WorktreeReviewDecision,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeConflictDisposition {
    None,
    HumanRequired,
    ReplanRequested,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorktreeConflictDetails {
    pub check_id: String,
    pub evidence_id: String,
    pub target_ref: String,
    pub target_commit: String,
    pub head_commit: String,
    pub conflict_free: bool,
    pub disposition: WorktreeConflictDisposition,
    pub conflicting_paths_count: u32,
    pub conflicts_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouteDecisionDetails {
    pub request_id: String,
    pub policy_id: String,
    pub policy_revision: u64,
    pub evaluated_at_timestamp_ms: u64,
    pub observed_event_count: u64,
    pub request: RouteRequest,
    pub selected_candidate_id: Option<String>,
    pub selected_score_basis_points: Option<u16>,
    pub estimated_cost_microusd: Option<u64>,
    pub evaluated_candidates: u32,
    pub eligible_candidates: u32,
    #[serde(default)]
    pub home_locked: bool,
    #[serde(default)]
    pub model_locked: bool,
    #[serde(default)]
    pub sensitive_directory: bool,
    #[serde(default)]
    pub reasons: Vec<String>,
    #[serde(default)]
    pub rejection_summary: Vec<String>,
    #[serde(default)]
    pub candidates: Vec<RouteCandidateDecisionSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouteCandidateDecisionSnapshot {
    pub candidate_id: String,
    pub identity: ExecutionIdentity,
    pub eligible: bool,
    pub total_score_basis_points: Option<u16>,
    pub estimated_cost_microusd: u64,
    pub historical_attempts: u64,
    pub historical_success_basis_points: u16,
    pub active_attempts: u64,
    pub quota_remaining_basis_points: Option<u16>,
    #[serde(default)]
    pub components: Vec<RouteScoreSnapshot>,
    #[serde(default)]
    pub reasons: Vec<String>,
    #[serde(default)]
    pub rejection_reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouteScoreSnapshot {
    pub name: String,
    pub score_basis_points: u16,
    pub weight_basis_points: u16,
    pub weighted_points: u16,
    pub reason: String,
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
    #[serde(default)]
    pub details: EventDetails,
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
            (
                "task.label",
                self.details.task.as_ref().map(|task| task.label.as_str()),
            ),
            (
                "task.kind",
                self.details
                    .task
                    .as_ref()
                    .and_then(|task| task.kind.as_deref()),
            ),
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
        if !self.safety.includes_local_paths
            && event_text_values(self).any(contains_obvious_local_path)
        {
            bail!(
                "event {} contains an apparent local path but safety.includesLocalPaths is false",
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
        validate_event_details(self)?;
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
    pub route_decisions: u64,
    pub worktrees_prepared: u64,
    pub worktree_evidence: u64,
    pub worktree_reviews: u64,
    pub worktree_conflict_checks: u64,
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

    pub fn load_verified(&self) -> Result<Vec<ObservabilityEvent>> {
        let events = self.load()?;
        verify_events(&events)?;
        Ok(events)
    }

    pub fn append(&self, incoming: &[ObservabilityEvent]) -> Result<ObservabilityAppendReport> {
        if incoming.is_empty() {
            bail!("at least one observability event is required");
        }
        let incoming = incoming.to_vec();
        self.append_checked(move |_| Ok(incoming))
    }

    pub fn append_checked(
        &self,
        build: impl FnOnce(&[ObservabilityEvent]) -> Result<Vec<ObservabilityEvent>>,
    ) -> Result<ObservabilityAppendReport> {
        let parent = self
            .path
            .parent()
            .context("observability store path has no parent directory")?;
        let parent_existed = parent.exists();
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
        if !parent_existed {
            secure_directory(parent)?;
        }
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
        let incoming = build(&existing)?;
        if incoming.is_empty() {
            bail!("at least one observability event is required");
        }
        let mut combined = existing.clone();
        combined.extend_from_slice(&incoming);
        verify_events(&combined)?;
        file.seek(SeekFrom::End(0))?;
        for event in &incoming {
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
        let events = self.load_verified()?;
        let filtered = filter.filter(&events);
        summarize(&filtered, &self.path)
    }

    pub fn export(&self, filter: &ObservabilityFilter) -> Result<ObservabilityExport> {
        let events = self.load_verified()?;
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
        "schema_version,event_id,timestamp_ms,event_type,status,task_id,run_id,attempt_id,thread_id,home_id,home_alias,account_id,provider,model,input_tokens,output_tokens,cached_input_tokens,cache_hits,cache_misses,duration_ms,estimated_cost_microusd,retries,failure_code,failure_phase,failure_reason,task_label,task_kind,budget_max_total_tokens,budget_max_duration_ms,budget_max_cost_microusd,budget_max_attempts,route_reason,route_decision_id,routing_request_id,routing_policy_id,routing_evaluated_at_timestamp_ms,routing_observed_event_count,routing_task_kind,routing_context_tokens,routing_output_tokens,routing_required_capabilities,routing_preferred_specialties,routing_required_security_domain,routing_locked_home,routing_locked_model,routing_max_estimated_cost_microusd,routing_allow_degraded,routing_selected_candidate_id,routing_score_basis_points,transition_kind,from_attempt_id,worktree_repository_root,worktree_path,worktree_branch,worktree_base_ref,worktree_base_commit,worktree_head_commit,evidence_id,evidence_commit_count,evidence_diff_sha256,evidence_patch_path,evidence_patch_bytes,evidence_changed_files,evidence_insertions,evidence_deletions,evidence_clean,evidence_tests_succeeded,evidence_test_label,evidence_test_exit_code,evidence_test_duration_ms,evidence_test_output_sha256,evidence_test_log_path,review_id,review_evidence_id,review_decision,review_reason,conflict_check_id,conflict_evidence_id,conflict_target_ref,conflict_target_commit,conflict_head_commit,conflict_free,conflict_disposition,conflicting_paths_count,conflicts_sha256,artifact_id,verification_id,target_artifact_id,final_artifact_ids\n",
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
            event
                .details
                .task
                .as_ref()
                .map(|task| task.label.clone())
                .unwrap_or_default(),
            event
                .details
                .task
                .as_ref()
                .and_then(|task| task.kind.clone())
                .unwrap_or_default(),
            event
                .details
                .budget
                .as_ref()
                .and_then(|budget| budget.max_total_tokens)
                .map(|value| value.to_string())
                .unwrap_or_default(),
            event
                .details
                .budget
                .as_ref()
                .and_then(|budget| budget.max_duration_ms)
                .map(|value| value.to_string())
                .unwrap_or_default(),
            event
                .details
                .budget
                .as_ref()
                .and_then(|budget| budget.max_cost_microusd)
                .map(|value| value.to_string())
                .unwrap_or_default(),
            event
                .details
                .budget
                .as_ref()
                .and_then(|budget| budget.max_attempts)
                .map(|value| value.to_string())
                .unwrap_or_default(),
            option(&event.details.route_reason),
            option(&event.details.route_decision_id),
            event
                .details
                .routing
                .as_ref()
                .map(|routing| routing.request_id.clone())
                .unwrap_or_default(),
            event
                .details
                .routing
                .as_ref()
                .map(|routing| routing.policy_id.clone())
                .unwrap_or_default(),
            event
                .details
                .routing
                .as_ref()
                .map(|routing| routing.evaluated_at_timestamp_ms.to_string())
                .unwrap_or_default(),
            event
                .details
                .routing
                .as_ref()
                .map(|routing| routing.observed_event_count.to_string())
                .unwrap_or_default(),
            event
                .details
                .routing
                .as_ref()
                .map(|routing| json_name(&routing.request.task_kind))
                .unwrap_or_default(),
            event
                .details
                .routing
                .as_ref()
                .map(|routing| routing.request.estimated_context_tokens.to_string())
                .unwrap_or_default(),
            event
                .details
                .routing
                .as_ref()
                .map(|routing| routing.request.estimated_output_tokens.to_string())
                .unwrap_or_default(),
            event
                .details
                .routing
                .as_ref()
                .map(|routing| routing.request.required_capabilities.join(";"))
                .unwrap_or_default(),
            event
                .details
                .routing
                .as_ref()
                .map(|routing| routing.request.preferred_specialties.join(";"))
                .unwrap_or_default(),
            event
                .details
                .routing
                .as_ref()
                .and_then(|routing| routing.request.required_security_domain.clone())
                .unwrap_or_default(),
            event
                .details
                .routing
                .as_ref()
                .and_then(|routing| routing.request.locked_home.clone())
                .unwrap_or_default(),
            event
                .details
                .routing
                .as_ref()
                .and_then(|routing| routing.request.locked_model.clone())
                .unwrap_or_default(),
            event
                .details
                .routing
                .as_ref()
                .and_then(|routing| routing.request.max_estimated_cost_microusd)
                .map(|value| value.to_string())
                .unwrap_or_default(),
            event
                .details
                .routing
                .as_ref()
                .map(|routing| routing.request.allow_degraded.to_string())
                .unwrap_or_default(),
            event
                .details
                .routing
                .as_ref()
                .and_then(|routing| routing.selected_candidate_id.clone())
                .unwrap_or_default(),
            event
                .details
                .routing
                .as_ref()
                .and_then(|routing| routing.selected_score_basis_points)
                .map(|value| value.to_string())
                .unwrap_or_default(),
            event
                .details
                .transition
                .as_ref()
                .map(|transition| json_name(&transition.kind))
                .unwrap_or_default(),
            event
                .details
                .transition
                .as_ref()
                .and_then(|transition| transition.from_attempt_id.clone())
                .unwrap_or_default(),
            event
                .details
                .worktree
                .as_ref()
                .map(|worktree| worktree.repository_root.clone())
                .unwrap_or_default(),
            event
                .details
                .worktree
                .as_ref()
                .map(|worktree| worktree.worktree_path.clone())
                .unwrap_or_default(),
            event
                .details
                .worktree
                .as_ref()
                .map(|worktree| worktree.branch.clone())
                .unwrap_or_default(),
            event
                .details
                .worktree
                .as_ref()
                .map(|worktree| worktree.base_ref.clone())
                .unwrap_or_default(),
            event
                .details
                .worktree
                .as_ref()
                .map(|worktree| worktree.base_commit.clone())
                .unwrap_or_default(),
            event
                .details
                .worktree
                .as_ref()
                .map(|worktree| worktree.head_commit.clone())
                .unwrap_or_default(),
            event
                .details
                .worktree_evidence
                .as_ref()
                .map(|evidence| evidence.evidence_id.clone())
                .unwrap_or_default(),
            event
                .details
                .worktree_evidence
                .as_ref()
                .map(|evidence| evidence.commit_count.to_string())
                .unwrap_or_default(),
            event
                .details
                .worktree_evidence
                .as_ref()
                .map(|evidence| evidence.diff_sha256.clone())
                .unwrap_or_default(),
            event
                .details
                .worktree_evidence
                .as_ref()
                .map(|evidence| evidence.patch_path.clone())
                .unwrap_or_default(),
            event
                .details
                .worktree_evidence
                .as_ref()
                .map(|evidence| evidence.patch_bytes.to_string())
                .unwrap_or_default(),
            event
                .details
                .worktree_evidence
                .as_ref()
                .map(|evidence| evidence.changed_files.to_string())
                .unwrap_or_default(),
            event
                .details
                .worktree_evidence
                .as_ref()
                .map(|evidence| evidence.insertions.to_string())
                .unwrap_or_default(),
            event
                .details
                .worktree_evidence
                .as_ref()
                .map(|evidence| evidence.deletions.to_string())
                .unwrap_or_default(),
            event
                .details
                .worktree_evidence
                .as_ref()
                .map(|evidence| evidence.worktree_clean.to_string())
                .unwrap_or_default(),
            event
                .details
                .worktree_evidence
                .as_ref()
                .map(|evidence| evidence.tests_succeeded.to_string())
                .unwrap_or_default(),
            event
                .details
                .worktree_evidence
                .as_ref()
                .map(|evidence| evidence.test_label.clone())
                .unwrap_or_default(),
            event
                .details
                .worktree_evidence
                .as_ref()
                .map(|evidence| evidence.test_exit_code.to_string())
                .unwrap_or_default(),
            event
                .details
                .worktree_evidence
                .as_ref()
                .map(|evidence| evidence.test_duration_ms.to_string())
                .unwrap_or_default(),
            event
                .details
                .worktree_evidence
                .as_ref()
                .map(|evidence| evidence.test_output_sha256.clone())
                .unwrap_or_default(),
            event
                .details
                .worktree_evidence
                .as_ref()
                .map(|evidence| evidence.test_log_path.clone())
                .unwrap_or_default(),
            event
                .details
                .worktree_review
                .as_ref()
                .map(|review| review.review_id.clone())
                .unwrap_or_default(),
            event
                .details
                .worktree_review
                .as_ref()
                .map(|review| review.evidence_id.clone())
                .unwrap_or_default(),
            event
                .details
                .worktree_review
                .as_ref()
                .map(|review| json_name(&review.decision))
                .unwrap_or_default(),
            event
                .details
                .worktree_review
                .as_ref()
                .map(|review| review.reason.clone())
                .unwrap_or_default(),
            event
                .details
                .worktree_conflict
                .as_ref()
                .map(|conflict| conflict.check_id.clone())
                .unwrap_or_default(),
            event
                .details
                .worktree_conflict
                .as_ref()
                .map(|conflict| conflict.evidence_id.clone())
                .unwrap_or_default(),
            event
                .details
                .worktree_conflict
                .as_ref()
                .map(|conflict| conflict.target_ref.clone())
                .unwrap_or_default(),
            event
                .details
                .worktree_conflict
                .as_ref()
                .map(|conflict| conflict.target_commit.clone())
                .unwrap_or_default(),
            event
                .details
                .worktree_conflict
                .as_ref()
                .map(|conflict| conflict.head_commit.clone())
                .unwrap_or_default(),
            event
                .details
                .worktree_conflict
                .as_ref()
                .map(|conflict| conflict.conflict_free.to_string())
                .unwrap_or_default(),
            event
                .details
                .worktree_conflict
                .as_ref()
                .map(|conflict| json_name(&conflict.disposition))
                .unwrap_or_default(),
            event
                .details
                .worktree_conflict
                .as_ref()
                .map(|conflict| conflict.conflicting_paths_count.to_string())
                .unwrap_or_default(),
            event
                .details
                .worktree_conflict
                .as_ref()
                .map(|conflict| conflict.conflicts_sha256.clone())
                .unwrap_or_default(),
            option(&event.details.artifact_id),
            option(&event.details.verification_id),
            option(&event.details.target_artifact_id),
            event.details.final_artifact_ids.join(";"),
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
            ObservabilityEventType::RouteDecided => {
                checked_add(&mut self.totals.route_decisions, 1, "route decision count")?
            }
            ObservabilityEventType::WorktreePrepared => checked_add(
                &mut self.totals.worktrees_prepared,
                1,
                "prepared worktree count",
            )?,
            ObservabilityEventType::WorktreeEvidenceRecorded => checked_add(
                &mut self.totals.worktree_evidence,
                1,
                "worktree evidence count",
            )?,
            ObservabilityEventType::WorktreeReviewRecorded => checked_add(
                &mut self.totals.worktree_reviews,
                1,
                "worktree review count",
            )?,
            ObservabilityEventType::WorktreeConflictChecked => checked_add(
                &mut self.totals.worktree_conflict_checks,
                1,
                "worktree conflict check count",
            )?,
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
        if !matches!(
            event.event_type,
            ObservabilityEventType::AttemptCompleted | ObservabilityEventType::AttemptFailed
        ) {
            return Ok(());
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
    terminal_attempts: BTreeMap<String, (ObservabilityStatus, bool)>,
    terminal_runs: BTreeSet<String>,
    artifacts: BTreeMap<String, (String, String, String)>,
    verifications: BTreeSet<String>,
    route_decisions: BTreeMap<String, (String, String, ExecutionIdentity)>,
    worktrees: BTreeMap<String, (String, String, String, ExecutionIdentity)>,
    worktree_evidence: BTreeMap<String, (String, String, String, String)>,
    worktree_reviews: BTreeSet<String>,
    worktree_conflict_checks: BTreeSet<String>,
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
    if let Some(run) = run_id {
        if indexes.terminal_runs.contains(run)
            && !matches!(
                event.event_type,
                ObservabilityEventType::QuotaSnapshot
                    | ObservabilityEventType::RateLimited
                    | ObservabilityEventType::AuthInvalid
                    | ObservabilityEventType::HomeHealth
            )
        {
            bail!(
                "event {} is appended after runId '{}' reached a terminal state",
                event.event_id,
                run
            );
        }
    }
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
            verify_attempt_transition(event, indexes, task, run)?;
            verify_route_decision_link(event, indexes, task, run)?;
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
            if let Some(attempt) = attempt_id {
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
            }
            let thread = required("threadId", thread_id)?;
            if indexes
                .threads
                .insert(thread.to_owned(), (task.to_owned(), run.to_owned()))
                .is_some()
            {
                bail!("threadId '{}' is linked more than once", thread);
            }
            if let Some(parent) = &event.trace.parent_thread_id {
                let expected = indexes.threads.get(parent).with_context(|| {
                    format!(
                        "event {} references unknown parentThreadId '{}'",
                        event.event_id, parent
                    )
                })?;
                if expected != &(task.to_owned(), run.to_owned()) {
                    bail!(
                        "event {} parentThreadId '{}' belongs to another task/run",
                        event.event_id,
                        parent
                    );
                }
            }
        }
        ObservabilityEventType::RouteDecided => {
            let (task, run) = verify_run_link(event, &indexes.runs)?;
            let routing = event
                .details
                .routing
                .as_ref()
                .context("route_decided is missing routing details")?;
            let observed_event_count = usize::try_from(routing.observed_event_count)
                .context("route decision observedEventCount exceeds usize")?;
            let prior_event_count = indexes.events.len().saturating_sub(1);
            if observed_event_count != prior_event_count {
                bail!(
                    "event {} route decision observed {} event(s), but {} preceded it",
                    event.event_id,
                    observed_event_count,
                    prior_event_count
                );
            }
            if indexes
                .route_decisions
                .insert(
                    event.event_id.clone(),
                    (task.to_owned(), run.to_owned(), event.identity.clone()),
                )
                .is_some()
            {
                bail!(
                    "route decision '{}' is recorded more than once",
                    event.event_id
                );
            }
        }
        ObservabilityEventType::WorktreePrepared => {
            let (task, run) = verify_run_link(event, &indexes.runs)?;
            let attempt = verify_attempt_link(event, indexes, task, run)?;
            if indexes.worktrees.contains_key(run) {
                bail!("runId '{}' already has a prepared worktree", run);
            }
            indexes.worktrees.insert(
                run.to_owned(),
                (
                    task.to_owned(),
                    run.to_owned(),
                    attempt.to_owned(),
                    event.identity.clone(),
                ),
            );
        }
        ObservabilityEventType::WorktreeEvidenceRecorded => {
            let (task, run) = verify_run_link(event, &indexes.runs)?;
            let attempt = verify_attempt_link(event, indexes, task, run)?;
            let worktree = indexes.worktrees.get(run).with_context(|| {
                format!(
                    "event {} records evidence before a worktree is prepared",
                    event.event_id
                )
            })?;
            if worktree.2 != attempt {
                bail!(
                    "event {} records evidence from a different attempt",
                    event.event_id
                );
            }
            if event.identity != worktree.3 {
                bail!(
                    "event {} evidence identity differs from the responsible identity",
                    event.event_id
                );
            }
            let evidence = event
                .details
                .worktree_evidence
                .as_ref()
                .context("worktree_evidence_recorded is missing details")?;
            if indexes
                .worktree_evidence
                .insert(
                    evidence.evidence_id.clone(),
                    (
                        task.to_owned(),
                        run.to_owned(),
                        attempt.to_owned(),
                        evidence.head_commit.clone(),
                    ),
                )
                .is_some()
            {
                bail!(
                    "worktree evidenceId '{}' is recorded more than once",
                    evidence.evidence_id
                );
            }
        }
        ObservabilityEventType::WorktreeReviewRecorded => {
            let (task, run) = verify_run_link(event, &indexes.runs)?;
            verify_attempt_link(event, indexes, task, run)?;
            let review = event
                .details
                .worktree_review
                .as_ref()
                .context("worktree_review_recorded is missing details")?;
            let evidence = indexes
                .worktree_evidence
                .get(&review.evidence_id)
                .with_context(|| {
                    format!(
                        "event {} references unknown evidenceId '{}'",
                        event.event_id, review.evidence_id
                    )
                })?;
            if evidence.0 != task || evidence.1 != run {
                bail!(
                    "event {} evidence belongs to another task/run",
                    event.event_id
                );
            }
            let responsible = &indexes
                .worktrees
                .get(run)
                .context("reviewed run has no prepared worktree")?
                .3;
            if event.identity.home_id == responsible.home_id {
                bail!(
                    "event {} reviewer Home must differ from the responsible Home",
                    event.event_id
                );
            }
            if event.identity.agent_role.as_deref() != Some("main_reviewer") {
                bail!(
                    "event {} reviewer identity must use agentRole 'main_reviewer'",
                    event.event_id
                );
            }
            if !indexes.worktree_reviews.insert(review.review_id.clone()) {
                bail!(
                    "worktree reviewId '{}' is recorded more than once",
                    review.review_id
                );
            }
        }
        ObservabilityEventType::WorktreeConflictChecked => {
            let (task, run) = verify_run_link(event, &indexes.runs)?;
            verify_attempt_link(event, indexes, task, run)?;
            let conflict = event
                .details
                .worktree_conflict
                .as_ref()
                .context("worktree_conflict_checked is missing details")?;
            let evidence = indexes
                .worktree_evidence
                .get(&conflict.evidence_id)
                .with_context(|| {
                    format!(
                        "event {} references unknown evidenceId '{}'",
                        event.event_id, conflict.evidence_id
                    )
                })?;
            if evidence.0 != task || evidence.1 != run || evidence.3 != conflict.head_commit {
                bail!(
                    "event {} conflict check does not match its evidence",
                    event.event_id
                );
            }
            if !indexes
                .worktree_conflict_checks
                .insert(conflict.check_id.clone())
            {
                bail!(
                    "worktree checkId '{}' is recorded more than once",
                    conflict.check_id
                );
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
            ) {
                let retryable = event
                    .failure
                    .as_ref()
                    .is_some_and(|failure| failure.retryable);
                if indexes
                    .terminal_attempts
                    .insert(attempt.to_owned(), (event.status, retryable))
                    .is_some()
                {
                    bail!("attemptId '{}' has more than one terminal event", attempt);
                }
            }
            if event.event_type == ObservabilityEventType::ArtifactCreated {
                let artifact = event
                    .details
                    .artifact_id
                    .as_deref()
                    .unwrap_or(&event.event_id);
                if indexes
                    .artifacts
                    .insert(
                        artifact.to_owned(),
                        (task.to_owned(), run.to_owned(), attempt.to_owned()),
                    )
                    .is_some()
                {
                    bail!("artifactId '{}' is created more than once", artifact);
                }
            }
            if event.event_type == ObservabilityEventType::VerificationCompleted {
                let verification = event
                    .details
                    .verification_id
                    .as_deref()
                    .unwrap_or(&event.event_id);
                if !indexes.verifications.insert(verification.to_owned()) {
                    bail!(
                        "verificationId '{}' is created more than once",
                        verification
                    );
                }
                if let Some(artifact) = &event.details.target_artifact_id {
                    let expected = indexes.artifacts.get(artifact).with_context(|| {
                        format!(
                            "event {} references unknown targetArtifactId '{}'",
                            event.event_id, artifact
                        )
                    })?;
                    if expected.0 != task || expected.1 != run {
                        bail!(
                            "event {} targetArtifactId '{}' belongs to another task/run",
                            event.event_id,
                            artifact
                        );
                    }
                }
            }
        }
        ObservabilityEventType::RunCompleted | ObservabilityEventType::RunFailed => {
            let (task, run) = verify_run_link(event, &indexes.runs)?;
            if indexes.attempts.iter().any(|(attempt, owner)| {
                owner == &(task.to_owned(), run.to_owned())
                    && !indexes.terminal_attempts.contains_key(attempt)
            }) {
                bail!("runId '{}' has active attempts", run);
            }
            if event.event_type == ObservabilityEventType::RunCompleted
                && !indexes
                    .terminal_attempts
                    .iter()
                    .any(|(attempt, (status, _))| {
                        *status == ObservabilityStatus::Succeeded
                            && indexes.attempts.get(attempt)
                                == Some(&(task.to_owned(), run.to_owned()))
                    })
            {
                bail!(
                    "runId '{}' cannot complete without a successful attempt",
                    run
                );
            }
            for artifact in &event.details.final_artifact_ids {
                let expected = indexes.artifacts.get(artifact).with_context(|| {
                    format!(
                        "event {} references unknown finalArtifactId '{}'",
                        event.event_id, artifact
                    )
                })?;
                if expected.0 != task || expected.1 != run {
                    bail!(
                        "event {} finalArtifactId '{}' belongs to another task/run",
                        event.event_id,
                        artifact
                    );
                }
            }
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

fn verify_attempt_link<'a>(
    event: &'a ObservabilityEvent,
    indexes: &'a TraceIndexes,
    task: &str,
    run: &str,
) -> Result<&'a str> {
    let attempt = required("attemptId", event.trace.attempt_id.as_deref())?;
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
    Ok(attempt)
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

fn verify_attempt_transition(
    event: &ObservabilityEvent,
    indexes: &TraceIndexes,
    task: &str,
    run: &str,
) -> Result<()> {
    let Some(transition) = &event.details.transition else {
        return Ok(());
    };
    match transition.kind {
        AttemptTransitionKind::Initial => {
            if transition.from_attempt_id.is_some() {
                bail!(
                    "event {} initial attempt cannot name fromAttemptId",
                    event.event_id
                );
            }
        }
        AttemptTransitionKind::Retry | AttemptTransitionKind::Migration => {
            let source = required(
                "details.transition.fromAttemptId",
                transition.from_attempt_id.as_deref(),
            )?;
            let owner = indexes.attempts.get(source).with_context(|| {
                format!(
                    "event {} references unknown fromAttemptId '{}'",
                    event.event_id, source
                )
            })?;
            if owner != &(task.to_owned(), run.to_owned()) {
                bail!(
                    "event {} fromAttemptId '{}' belongs to another task/run",
                    event.event_id,
                    source
                );
            }
            let (status, retryable) = indexes.terminal_attempts.get(source).with_context(|| {
                format!(
                    "event {} fromAttemptId '{}' is not terminal",
                    event.event_id, source
                )
            })?;
            if *status == ObservabilityStatus::Succeeded {
                bail!(
                    "event {} cannot continue from successful attempt '{}'",
                    event.event_id,
                    source
                );
            }
            if transition.kind == AttemptTransitionKind::Retry && !retryable {
                bail!(
                    "event {} cannot retry non-retryable attempt '{}'",
                    event.event_id,
                    source
                );
            }
        }
    }
    Ok(())
}

fn verify_route_decision_link(
    event: &ObservabilityEvent,
    indexes: &TraceIndexes,
    task: &str,
    run: &str,
) -> Result<()> {
    let Some(route_decision_id) = &event.details.route_decision_id else {
        return Ok(());
    };
    let (owner_task, owner_run, selected_identity) = indexes
        .route_decisions
        .get(route_decision_id)
        .with_context(|| {
            format!(
                "event {} references unknown routeDecisionId '{}'",
                event.event_id, route_decision_id
            )
        })?;
    if owner_task != task || owner_run != run {
        bail!(
            "event {} routeDecisionId '{}' belongs to another task/run",
            event.event_id,
            route_decision_id
        );
    }
    for (name, selected, actual) in [
        (
            "homeId",
            selected_identity.home_id.as_deref(),
            event.identity.home_id.as_deref(),
        ),
        (
            "accountId",
            selected_identity.account_id.as_deref(),
            event.identity.account_id.as_deref(),
        ),
        (
            "model",
            selected_identity.model.as_deref(),
            event.identity.model.as_deref(),
        ),
    ] {
        if selected != actual {
            bail!(
                "event {} identity {name} does not match routeDecisionId '{}'",
                event.event_id,
                route_decision_id
            );
        }
    }
    Ok(())
}

fn validate_event_details(event: &ObservabilityEvent) -> Result<()> {
    let details = &event.details;
    if let Some(task) = &details.task {
        validate_label("details.task.label", &task.label)?;
        if let Some(kind) = &task.kind {
            validate_label("details.task.kind", kind)?;
        }
        if event.event_type != ObservabilityEventType::TaskCreated {
            bail!("event {} task details require task_created", event.event_id);
        }
    }
    if let Some(budget) = &details.budget {
        if event.event_type != ObservabilityEventType::RunStarted {
            bail!("event {} budget requires run_started", event.event_id);
        }
        if budget.max_total_tokens.is_none()
            && budget.max_duration_ms.is_none()
            && budget.max_cost_microusd.is_none()
            && budget.max_attempts.is_none()
        {
            bail!("event {} budget has no limits", event.event_id);
        }
        if budget.max_total_tokens == Some(0)
            || budget.max_duration_ms == Some(0)
            || budget.max_cost_microusd == Some(0)
            || budget.max_attempts == Some(0)
        {
            bail!("event {} budget limits must be positive", event.event_id);
        }
    }
    if let Some(reason) = &details.route_reason {
        let reason = reason.trim();
        if reason.is_empty() || reason.len() > 2048 || contains_obvious_secret(reason) {
            bail!("event {} has an invalid route reason", event.event_id);
        }
        if event.event_type != ObservabilityEventType::AttemptStarted {
            bail!(
                "event {} routeReason requires attempt_started",
                event.event_id
            );
        }
    }
    if let Some(route_decision_id) = &details.route_decision_id {
        validate_identifier("details.routeDecisionId", route_decision_id)?;
        if event.event_type != ObservabilityEventType::AttemptStarted {
            bail!(
                "event {} routeDecisionId requires attempt_started",
                event.event_id
            );
        }
    }
    if let Some(routing) = &details.routing {
        if event.event_type != ObservabilityEventType::RouteDecided {
            bail!(
                "event {} routing details require route_decided",
                event.event_id
            );
        }
        validate_identifier("details.routing.requestId", &routing.request_id)?;
        validate_identifier("details.routing.policyId", &routing.policy_id)?;
        routing.request.validate()?;
        if routing.request_id != routing.request.request_id {
            bail!(
                "event {} routing requestId disagrees with its request snapshot",
                event.event_id
            );
        }
        if routing.home_locked != routing.request.locked_home.is_some()
            || routing.model_locked != routing.request.locked_model.is_some()
            || routing.sensitive_directory != routing.request.sensitive_directory
        {
            bail!(
                "event {} routing lock flags disagree with its request snapshot",
                event.event_id
            );
        }
        if let Some(candidate_id) = &routing.selected_candidate_id {
            validate_identifier("details.routing.selectedCandidateId", candidate_id)?;
        }
        if routing
            .selected_score_basis_points
            .is_some_and(|score| score > 10_000)
        {
            bail!(
                "event {} selected route score exceeds 10000",
                event.event_id
            );
        }
        if routing.eligible_candidates > routing.evaluated_candidates {
            bail!(
                "event {} has more eligible than evaluated route candidates",
                event.event_id
            );
        }
        validate_route_texts(event, "details.routing.reasons", &routing.reasons)?;
        validate_route_texts(
            event,
            "details.routing.rejectionSummary",
            &routing.rejection_summary,
        )?;
        validate_route_candidates(event, routing)?;
        let selected = routing.selected_candidate_id.is_some()
            && routing.selected_score_basis_points.is_some()
            && routing.estimated_cost_microusd.is_some();
        if event.status == ObservabilityStatus::Succeeded && !selected {
            bail!(
                "event {} successful route decision requires a selected candidate, score, and cost",
                event.event_id
            );
        }
        if event.status == ObservabilityStatus::Failed && routing.selected_candidate_id.is_some() {
            bail!(
                "event {} failed route decision cannot select a candidate",
                event.event_id
            );
        }
    }
    if let Some(transition) = &details.transition {
        if event.event_type != ObservabilityEventType::AttemptStarted {
            bail!(
                "event {} transition requires attempt_started",
                event.event_id
            );
        }
        if let Some(source) = &transition.from_attempt_id {
            validate_identifier("details.transition.fromAttemptId", source)?;
        }
    }
    if let Some(worktree) = &details.worktree {
        if event.event_type != ObservabilityEventType::WorktreePrepared {
            bail!(
                "event {} worktree details require worktree_prepared",
                event.event_id
            );
        }
        validate_local_path("details.worktree.repositoryRoot", &worktree.repository_root)?;
        validate_local_path("details.worktree.worktreePath", &worktree.worktree_path)?;
        validate_label("details.worktree.branch", &worktree.branch)?;
        validate_label("details.worktree.baseRef", &worktree.base_ref)?;
        validate_git_commit("details.worktree.baseCommit", &worktree.base_commit)?;
        validate_git_commit("details.worktree.headCommit", &worktree.head_commit)?;
        if worktree.base_commit != worktree.head_commit {
            bail!(
                "event {} prepared worktree must begin at its base commit",
                event.event_id
            );
        }
    }
    if let Some(evidence) = &details.worktree_evidence {
        if event.event_type != ObservabilityEventType::WorktreeEvidenceRecorded {
            bail!(
                "event {} worktreeEvidence requires worktree_evidence_recorded",
                event.event_id
            );
        }
        validate_identifier("details.worktreeEvidence.evidenceId", &evidence.evidence_id)?;
        validate_git_commit("details.worktreeEvidence.baseCommit", &evidence.base_commit)?;
        validate_git_commit("details.worktreeEvidence.headCommit", &evidence.head_commit)?;
        validate_sha256("details.worktreeEvidence.diffSha256", &evidence.diff_sha256)?;
        validate_local_path("details.worktreeEvidence.patchPath", &evidence.patch_path)?;
        validate_label("details.worktreeEvidence.testLabel", &evidence.test_label)?;
        validate_sha256(
            "details.worktreeEvidence.testOutputSha256",
            &evidence.test_output_sha256,
        )?;
        validate_local_path(
            "details.worktreeEvidence.testLogPath",
            &evidence.test_log_path,
        )?;
        if evidence.commit_count == 0 || evidence.base_commit == evidence.head_commit {
            bail!(
                "event {} worktree evidence requires at least one commit",
                event.event_id
            );
        }
        if evidence.patch_bytes == 0 || evidence.changed_files == 0 {
            bail!(
                "event {} worktree evidence requires a non-empty patch",
                event.event_id
            );
        }
        if evidence.tests_succeeded != (evidence.test_exit_code == 0) {
            bail!(
                "event {} test success and exit code disagree",
                event.event_id
            );
        }
    }
    if let Some(review) = &details.worktree_review {
        if event.event_type != ObservabilityEventType::WorktreeReviewRecorded {
            bail!(
                "event {} worktreeReview requires worktree_review_recorded",
                event.event_id
            );
        }
        validate_identifier("details.worktreeReview.reviewId", &review.review_id)?;
        validate_identifier("details.worktreeReview.evidenceId", &review.evidence_id)?;
        validate_long_text("details.worktreeReview.reason", &review.reason, false)?;
    }
    if let Some(conflict) = &details.worktree_conflict {
        if event.event_type != ObservabilityEventType::WorktreeConflictChecked {
            bail!(
                "event {} worktreeConflict requires worktree_conflict_checked",
                event.event_id
            );
        }
        validate_identifier("details.worktreeConflict.checkId", &conflict.check_id)?;
        validate_identifier("details.worktreeConflict.evidenceId", &conflict.evidence_id)?;
        validate_label("details.worktreeConflict.targetRef", &conflict.target_ref)?;
        validate_git_commit(
            "details.worktreeConflict.targetCommit",
            &conflict.target_commit,
        )?;
        validate_git_commit("details.worktreeConflict.headCommit", &conflict.head_commit)?;
        validate_sha256(
            "details.worktreeConflict.conflictsSha256",
            &conflict.conflicts_sha256,
        )?;
        let disposition_matches = if conflict.conflict_free {
            conflict.disposition == WorktreeConflictDisposition::None
                && conflict.conflicting_paths_count == 0
        } else {
            matches!(
                conflict.disposition,
                WorktreeConflictDisposition::HumanRequired
                    | WorktreeConflictDisposition::ReplanRequested
            ) && conflict.conflicting_paths_count > 0
        };
        if !disposition_matches {
            bail!(
                "event {} conflict state and disposition disagree",
                event.event_id
            );
        }
    }
    for (name, value, expected_type) in [
        (
            "details.artifactId",
            details.artifact_id.as_deref(),
            ObservabilityEventType::ArtifactCreated,
        ),
        (
            "details.verificationId",
            details.verification_id.as_deref(),
            ObservabilityEventType::VerificationCompleted,
        ),
        (
            "details.targetArtifactId",
            details.target_artifact_id.as_deref(),
            ObservabilityEventType::VerificationCompleted,
        ),
    ] {
        if let Some(value) = value {
            validate_identifier(name, value)?;
            if event.event_type != expected_type {
                bail!(
                    "event {} {} is not valid for this event type",
                    event.event_id,
                    name
                );
            }
        }
    }
    if !details.final_artifact_ids.is_empty() {
        if event.event_type != ObservabilityEventType::RunCompleted {
            bail!(
                "event {} finalArtifactIds require run_completed",
                event.event_id
            );
        }
        let mut unique = BTreeSet::new();
        for artifact in &details.final_artifact_ids {
            validate_identifier("details.finalArtifactIds", artifact)?;
            if !unique.insert(artifact) {
                bail!(
                    "event {} repeats finalArtifactId '{}'",
                    event.event_id,
                    artifact
                );
            }
        }
    }
    Ok(())
}

fn validate_route_candidates(
    event: &ObservabilityEvent,
    routing: &RouteDecisionDetails,
) -> Result<()> {
    if routing.evaluated_at_timestamp_ms > event.timestamp_ms {
        bail!(
            "event {} route evaluation timestamp is after the recorded decision",
            event.event_id
        );
    }
    if routing.candidates.len() > 256 {
        bail!("event {} has too many route candidates", event.event_id);
    }
    if routing.candidates.len() != routing.evaluated_candidates as usize {
        bail!(
            "event {} routing candidate snapshot count does not match evaluatedCandidates",
            event.event_id
        );
    }
    let eligible_count = routing
        .candidates
        .iter()
        .filter(|candidate| candidate.eligible)
        .count();
    if eligible_count != routing.eligible_candidates as usize {
        bail!(
            "event {} routing eligible snapshot count does not match eligibleCandidates",
            event.event_id
        );
    }
    let mut candidate_ids = BTreeSet::new();
    for candidate in &routing.candidates {
        validate_identifier(
            "details.routing.candidates.candidateId",
            &candidate.candidate_id,
        )?;
        if !candidate_ids.insert(candidate.candidate_id.as_str()) {
            bail!(
                "event {} repeats route candidate '{}'",
                event.event_id,
                candidate.candidate_id
            );
        }
        required(
            "details.routing.candidates.identity.homeId",
            candidate.identity.home_id.as_deref(),
        )?;
        required(
            "details.routing.candidates.identity.model",
            candidate.identity.model.as_deref(),
        )?;
        if candidate.eligible != candidate.total_score_basis_points.is_some() {
            bail!(
                "event {} route candidate '{}' eligibility and score disagree",
                event.event_id,
                candidate.candidate_id
            );
        }
        if candidate
            .total_score_basis_points
            .is_some_and(|score| score > 10_000)
            || candidate.historical_success_basis_points > 10_000
            || candidate
                .quota_remaining_basis_points
                .is_some_and(|quota| quota > 10_000)
        {
            bail!(
                "event {} route candidate '{}' has a basis-point value above 10000",
                event.event_id,
                candidate.candidate_id
            );
        }
        if candidate.eligible && !candidate.rejection_reasons.is_empty() {
            bail!(
                "event {} eligible route candidate '{}' has rejection reasons",
                event.event_id,
                candidate.candidate_id
            );
        }
        if !candidate.eligible && candidate.rejection_reasons.is_empty() {
            bail!(
                "event {} rejected route candidate '{}' has no rejection reason",
                event.event_id,
                candidate.candidate_id
            );
        }
        validate_route_texts(
            event,
            "details.routing.candidates.rejectionReasons",
            &candidate.rejection_reasons,
        )?;
        validate_route_texts(
            event,
            "details.routing.candidates.reasons",
            &candidate.reasons,
        )?;
        let mut component_names = BTreeSet::new();
        let mut weight_total = 0_u32;
        let mut weighted_total = 0_u32;
        for component in &candidate.components {
            validate_label(
                "details.routing.candidates.components.name",
                &component.name,
            )?;
            if !component_names.insert(component.name.as_str()) {
                bail!(
                    "event {} route candidate '{}' repeats score component '{}'",
                    event.event_id,
                    candidate.candidate_id,
                    component.name
                );
            }
            if component.score_basis_points > 10_000
                || component.weight_basis_points > 10_000
                || component.weighted_points > 10_000
            {
                bail!(
                    "event {} route candidate '{}' has an invalid score component",
                    event.event_id,
                    candidate.candidate_id
                );
            }
            validate_route_texts(
                event,
                "details.routing.candidates.components.reason",
                std::slice::from_ref(&component.reason),
            )?;
            weight_total = weight_total.saturating_add(u32::from(component.weight_basis_points));
            weighted_total = weighted_total.saturating_add(u32::from(component.weighted_points));
        }
        if candidate.eligible
            && (candidate.components.is_empty()
                || weight_total != 10_000
                || candidate.total_score_basis_points != u16::try_from(weighted_total).ok())
        {
            bail!(
                "event {} route candidate '{}' score components do not reproduce its total",
                event.event_id,
                candidate.candidate_id
            );
        }
    }
    if let Some(selected_id) = &routing.selected_candidate_id {
        let selected = routing
            .candidates
            .iter()
            .find(|candidate| candidate.candidate_id == *selected_id)
            .with_context(|| {
                format!(
                    "event {} selected route candidate '{}' is missing from snapshot",
                    event.event_id, selected_id
                )
            })?;
        if !selected.eligible
            || selected.total_score_basis_points != routing.selected_score_basis_points
            || Some(selected.estimated_cost_microusd) != routing.estimated_cost_microusd
            || selected.identity != event.identity
        {
            bail!(
                "event {} selected route candidate snapshot disagrees with decision",
                event.event_id
            );
        }
    }
    Ok(())
}

fn validate_event_shape(event: &ObservabilityEvent) -> Result<()> {
    use ObservabilityEventType as Type;
    if matches!(
        event.event_type,
        Type::AttemptStarted
            | Type::WorktreePrepared
            | Type::WorktreeEvidenceRecorded
            | Type::WorktreeReviewRecorded
            | Type::WorktreeConflictChecked
            | Type::AttemptCompleted
            | Type::AttemptFailed
            | Type::ToolCallCompleted
            | Type::ArtifactCreated
            | Type::VerificationCompleted
    ) {
        required("homeId", event.identity.home_id.as_deref())?;
        required("model", event.identity.model.as_deref())?;
    }
    if event.event_type == Type::RouteDecided {
        required("taskId", event.trace.task_id.as_deref())?;
        required("runId", event.trace.run_id.as_deref())?;
        if event.details.routing.is_none() {
            bail!("event {} requires routing details", event.event_id);
        }
        if event.status == ObservabilityStatus::Succeeded {
            required("homeId", event.identity.home_id.as_deref())?;
            required("model", event.identity.model.as_deref())?;
        } else if event.status != ObservabilityStatus::Failed {
            bail!(
                "event {} route decision must use succeeded or failed status",
                event.event_id
            );
        }
    }
    for (event_type, present, name) in [
        (
            Type::WorktreePrepared,
            event.details.worktree.is_some(),
            "worktree",
        ),
        (
            Type::WorktreeEvidenceRecorded,
            event.details.worktree_evidence.is_some(),
            "worktreeEvidence",
        ),
        (
            Type::WorktreeReviewRecorded,
            event.details.worktree_review.is_some(),
            "worktreeReview",
        ),
        (
            Type::WorktreeConflictChecked,
            event.details.worktree_conflict.is_some(),
            "worktreeConflict",
        ),
    ] {
        if event.event_type == event_type && !present {
            bail!("event {} requires {name} details", event.event_id);
        }
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
    let requires_failure = matches!(
        event.event_type,
        Type::AttemptFailed | Type::RunFailed | Type::RateLimited | Type::AuthInvalid
    ) || (event.event_type == Type::RouteDecided
        && event.status == ObservabilityStatus::Failed);
    if requires_failure && event.failure.is_none() {
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

fn validate_long_text(name: &str, value: &str, allow_local_path: bool) -> Result<()> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.len() > 2_048
        || contains_obvious_secret(trimmed)
        || (!allow_local_path && contains_obvious_local_path(trimmed))
    {
        bail!("{name} is invalid");
    }
    Ok(())
}

fn validate_local_path(name: &str, value: &str) -> Result<()> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.len() > 4_096
        || !contains_obvious_local_path(trimmed)
        || contains_obvious_secret(trimmed)
        || trimmed
            .chars()
            .any(|character| matches!(character, '\r' | '\n'))
    {
        bail!("{name} is not a safe absolute local path");
    }
    Ok(())
}

fn validate_git_commit(name: &str, value: &str) -> Result<()> {
    if value.len() != 40 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("{name} must be a full 40-character hexadecimal Git commit");
    }
    Ok(())
}

fn validate_sha256(name: &str, value: &str) -> Result<()> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("{name} must be a 64-character hexadecimal SHA-256");
    }
    Ok(())
}

fn validate_route_texts(event: &ObservabilityEvent, name: &str, values: &[String]) -> Result<()> {
    if values.len() > 64 {
        bail!("event {} {name} has too many entries", event.event_id);
    }
    for value in values {
        let value = value.trim();
        if value.is_empty()
            || value.len() > 2048
            || contains_obvious_secret(value)
            || contains_obvious_local_path(value)
        {
            bail!("event {} has invalid {name} text", event.event_id);
        }
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

fn event_text_values(event: &ObservabilityEvent) -> impl Iterator<Item = &str> {
    [
        Some(event.event_id.as_str()),
        event.trace.task_id.as_deref(),
        event.trace.run_id.as_deref(),
        event.trace.attempt_id.as_deref(),
        event.trace.thread_id.as_deref(),
        event.trace.parent_thread_id.as_deref(),
        event.trace.fork_id.as_deref(),
        event.identity.home_id.as_deref(),
        event.identity.home_alias.as_deref(),
        event.identity.account_id.as_deref(),
        event.identity.provider.as_deref(),
        event.identity.model.as_deref(),
        event.identity.agent_role.as_deref(),
        event.tool_name.as_deref(),
        event.artifact_kind.as_deref(),
        event.verification_kind.as_deref(),
        event.details.task.as_ref().map(|task| task.label.as_str()),
        event
            .details
            .task
            .as_ref()
            .and_then(|task| task.kind.as_deref()),
        event.details.route_reason.as_deref(),
        event.details.route_decision_id.as_deref(),
        event
            .details
            .routing
            .as_ref()
            .map(|routing| routing.request_id.as_str()),
        event
            .details
            .routing
            .as_ref()
            .map(|routing| routing.policy_id.as_str()),
        event
            .details
            .routing
            .as_ref()
            .and_then(|routing| routing.selected_candidate_id.as_deref()),
        event
            .details
            .transition
            .as_ref()
            .and_then(|transition| transition.from_attempt_id.as_deref()),
        event
            .details
            .worktree
            .as_ref()
            .map(|worktree| worktree.repository_root.as_str()),
        event
            .details
            .worktree
            .as_ref()
            .map(|worktree| worktree.worktree_path.as_str()),
        event
            .details
            .worktree_evidence
            .as_ref()
            .map(|evidence| evidence.patch_path.as_str()),
        event
            .details
            .worktree_evidence
            .as_ref()
            .map(|evidence| evidence.test_log_path.as_str()),
        event.details.artifact_id.as_deref(),
        event.details.verification_id.as_deref(),
        event.details.target_artifact_id.as_deref(),
        event
            .failure
            .as_ref()
            .map(|failure| failure.reason.as_str()),
    ]
    .into_iter()
    .flatten()
}

fn contains_obvious_local_path(value: &str) -> bool {
    let normalized = value.replace('\\', "/");
    let lower = normalized.to_ascii_lowercase();
    lower.starts_with('/')
        || lower.contains(" /users/")
        || lower.contains(" /home/")
        || lower.contains(" /volumes/")
        || lower.contains(" /tmp/")
        || lower.contains(" /var/")
        || lower.starts_with("c:/")
        || lower.contains(" c:/")
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
            details: EventDetails::default(),
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
    fn aggregate_usage_uses_terminal_attempt_totals_without_double_counting_breakdowns() {
        let temp = TempDir::new().expect("temp");
        let store = ObservabilityStore::new(temp.path().join("events.jsonl"));
        let mut events = trace_fixture();
        let mut tool = events[2].clone();
        tool.event_id = "evt-tool".to_owned();
        tool.timestamp_ms = 4;
        tool.event_type = ObservabilityEventType::ToolCallCompleted;
        tool.status = ObservabilityStatus::Succeeded;
        tool.trace.thread_id = Some("thread-1".to_owned());
        tool.tool_name = Some("compiler".to_owned());
        tool.usage = events[4].usage.clone();
        events.insert(4, tool);
        store.append(&events).expect("append");

        let summary = store
            .summary(&ObservabilityFilter::default())
            .expect("summary");
        assert_eq!(summary.totals.tool_calls, 1);
        assert_eq!(summary.totals.total_tokens, 140);
        assert_eq!(summary.totals.duration_ms, 1500);
        assert_eq!(summary.totals.estimated_cost_microusd, 1200);
    }

    #[cfg(unix)]
    #[test]
    fn append_does_not_change_permissions_of_an_existing_parent_directory() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TempDir::new().expect("temp");
        fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o755))
            .expect("set parent permissions");
        let store = ObservabilityStore::new(temp.path().join("events.jsonl"));
        store
            .append(&[trace_fixture()[0].clone()])
            .expect("append into existing parent");
        let parent_mode = fs::metadata(temp.path())
            .expect("parent metadata")
            .permissions()
            .mode()
            & 0o777;
        let file_mode = fs::metadata(store.path())
            .expect("file metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(parent_mode, 0o755);
        assert_eq!(file_mode, 0o600);
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
            reason: "Authorization: Bearer [redacted-test-value]".to_owned(),
            retryable: true,
        });
        assert!(failed
            .validate()
            .expect_err("secret")
            .to_string()
            .contains("secret"));

        let mut local_path = trace_fixture()[0].clone();
        local_path.details.task = Some(TaskDescriptor {
            label: "Analyze /Users/example/private.pdf".to_owned(),
            kind: Some("document".to_owned()),
        });
        assert!(local_path
            .validate()
            .expect_err("unreported local path")
            .to_string()
            .contains("includesLocalPaths"));
        local_path.safety.includes_local_paths = true;
        local_path
            .validate()
            .expect("reported local path is allowed");
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
        assert!(csv.contains(
            "route_reason,route_decision_id,routing_request_id,routing_policy_id,routing_evaluated_at_timestamp_ms,routing_observed_event_count,routing_task_kind,routing_context_tokens,routing_output_tokens,routing_required_capabilities,routing_preferred_specialties"
        ));
    }
}
