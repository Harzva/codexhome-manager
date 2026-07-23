use crate::{
    AttemptTransition, AttemptTransitionKind, EventDetails, EventSafety, ExecutionIdentity,
    FailureInfo, ObservabilityEvent, ObservabilityEventType, ObservabilityStatus,
    ObservabilityStore, RouteCandidateDecisionSnapshot, RouteDecision, RouteDecisionDetails,
    RouteEngine, RouteHomeState, RoutePolicy, RouteRequest, RouteScoreSnapshot, RunBudget,
    SafetySummary, TaskDescriptor, TraceContext, UsageDelta, OBSERVABILITY_EVENT_SCHEMA_VERSION,
};
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub const AGENT_RUN_REPORT_SCHEMA_VERSION: &str = "codexhome.agent-runs.v1";
pub const AGENT_RUN_MUTATION_SCHEMA_VERSION: &str = "codexhome.agent-run-mutation.v1";

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone)]
pub enum AgentRunAction {
    CreateTask {
        task_id: String,
        label: String,
        kind: Option<String>,
    },
    StartRun {
        task_id: String,
        run_id: String,
        budget: Option<RunBudget>,
    },
    StartAttempt {
        run_id: String,
        attempt_id: String,
        identity: ExecutionIdentity,
        transition: AttemptTransition,
        route_reason: Option<String>,
        route_decision_id: Option<String>,
    },
    RecordRouteDecision {
        run_id: String,
        decision: Box<RouteDecision>,
    },
    CompleteAttempt {
        run_id: String,
        attempt_id: String,
        thread_id: Option<String>,
        usage: UsageDelta,
    },
    FailAttempt {
        run_id: String,
        attempt_id: String,
        thread_id: Option<String>,
        status: ObservabilityStatus,
        usage: UsageDelta,
        failure: FailureInfo,
    },
    LinkThread {
        run_id: String,
        attempt_id: Option<String>,
        thread_id: String,
        parent_thread_id: Option<String>,
        fork_id: Option<String>,
    },
    RecordTool {
        run_id: String,
        attempt_id: String,
        thread_id: Option<String>,
        tool_name: String,
        usage: UsageDelta,
    },
    RecordArtifact {
        run_id: String,
        attempt_id: String,
        thread_id: Option<String>,
        artifact_id: String,
        artifact_kind: String,
    },
    RecordVerification {
        run_id: String,
        attempt_id: String,
        thread_id: Option<String>,
        verification_id: String,
        verification_kind: String,
        target_artifact_id: Option<String>,
        succeeded: bool,
        duration_ms: u64,
    },
    CompleteRun {
        run_id: String,
        final_artifact_ids: Vec<String>,
    },
    FailRun {
        run_id: String,
        status: ObservabilityStatus,
        failure: FailureInfo,
    },
}

impl AgentRunAction {
    fn name(&self) -> &'static str {
        match self {
            Self::CreateTask { .. } => "create_task",
            Self::StartRun { .. } => "start_run",
            Self::StartAttempt { transition, .. } => match transition.kind {
                AttemptTransitionKind::Initial => "start_attempt",
                AttemptTransitionKind::Retry => "retry_attempt",
                AttemptTransitionKind::Migration => "migrate_attempt",
            },
            Self::RecordRouteDecision { .. } => "record_route_decision",
            Self::CompleteAttempt { .. } => "complete_attempt",
            Self::FailAttempt { .. } => "fail_attempt",
            Self::LinkThread { .. } => "link_thread",
            Self::RecordTool { .. } => "record_tool",
            Self::RecordArtifact { .. } => "record_artifact",
            Self::RecordVerification { .. } => "record_verification",
            Self::CompleteRun { .. } => "complete_run",
            Self::FailRun { status, .. } => match status {
                ObservabilityStatus::Cancelled => "cancel_run",
                ObservabilityStatus::TimedOut => "timeout_run",
                _ => "fail_run",
            },
        }
    }

    fn task_run_attempt(
        &self,
        report: &AgentRunReport,
    ) -> Result<(String, Option<String>, Option<String>)> {
        match self {
            Self::CreateTask { task_id, .. } => Ok((task_id.clone(), None, None)),
            Self::StartRun {
                task_id, run_id, ..
            } => Ok((task_id.clone(), Some(run_id.clone()), None)),
            Self::StartAttempt {
                run_id, attempt_id, ..
            }
            | Self::CompleteAttempt {
                run_id, attempt_id, ..
            }
            | Self::FailAttempt {
                run_id, attempt_id, ..
            }
            | Self::RecordTool {
                run_id, attempt_id, ..
            }
            | Self::RecordArtifact {
                run_id, attempt_id, ..
            }
            | Self::RecordVerification {
                run_id, attempt_id, ..
            } => {
                let run = report.run(run_id)?;
                Ok((
                    run.task_id.clone(),
                    Some(run_id.clone()),
                    Some(attempt_id.clone()),
                ))
            }
            Self::RecordRouteDecision { run_id, .. } => {
                let run = report.run(run_id)?;
                Ok((run.task_id.clone(), Some(run_id.clone()), None))
            }
            Self::LinkThread {
                run_id, attempt_id, ..
            } => {
                let run = report.run(run_id)?;
                Ok((
                    run.task_id.clone(),
                    Some(run_id.clone()),
                    attempt_id.clone(),
                ))
            }
            Self::CompleteRun { run_id, .. } | Self::FailRun { run_id, .. } => {
                let run = report.run(run_id)?;
                Ok((run.task_id.clone(), Some(run_id.clone()), None))
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskLifecycleStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunLifecycleStatus {
    Running,
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AttemptLifecycleStatus {
    Running,
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTaskView {
    pub task_id: String,
    pub label: String,
    pub kind: Option<String>,
    pub created_at_ms: u64,
    pub status: TaskLifecycleStatus,
    pub run_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentAttemptView {
    pub attempt_id: String,
    pub ordinal: usize,
    pub status: AttemptLifecycleStatus,
    pub started_at_ms: u64,
    pub finished_at_ms: Option<u64>,
    pub identity: ExecutionIdentity,
    pub transition: AttemptTransition,
    pub route_reason: Option<String>,
    pub route_decision_id: Option<String>,
    pub thread_ids: Vec<String>,
    pub usage: UsageDelta,
    pub failure: Option<FailureInfo>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRouteDecisionView {
    pub event_id: String,
    pub request_id: String,
    pub policy_id: String,
    pub policy_revision: u64,
    pub evaluated_at_timestamp_ms: u64,
    pub observed_event_count: u64,
    pub request: RouteRequest,
    pub decided_at_ms: u64,
    pub succeeded: bool,
    pub identity: ExecutionIdentity,
    pub selected_candidate_id: Option<String>,
    pub selected_score_basis_points: Option<u16>,
    pub estimated_cost_microusd: Option<u64>,
    pub evaluated_candidates: u32,
    pub eligible_candidates: u32,
    pub home_locked: bool,
    pub model_locked: bool,
    pub sensitive_directory: bool,
    pub reasons: Vec<String>,
    pub rejection_summary: Vec<String>,
    pub candidates: Vec<RouteCandidateDecisionSnapshot>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentArtifactView {
    pub artifact_id: String,
    pub artifact_kind: String,
    pub attempt_id: String,
    pub thread_id: Option<String>,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentVerificationView {
    pub verification_id: String,
    pub verification_kind: String,
    pub attempt_id: String,
    pub thread_id: Option<String>,
    pub target_artifact_id: Option<String>,
    pub succeeded: bool,
    pub duration_ms: u64,
    pub completed_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RunConsumption {
    pub attempts: usize,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub cached_input_tokens: u64,
    pub duration_ms: u64,
    pub estimated_cost_microusd: u64,
    pub retries: u64,
    pub failed_attempt_tokens: u64,
    pub failed_attempt_duration_ms: u64,
    pub failed_attempt_cost_microusd: u64,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RunBudgetState {
    pub within_budget: bool,
    pub exhausted_dimensions: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRecovery {
    pub can_retry: bool,
    pub can_migrate: bool,
    pub recommended_action: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRunView {
    pub run_id: String,
    pub task_id: String,
    pub status: RunLifecycleStatus,
    pub started_at_ms: u64,
    pub finished_at_ms: Option<u64>,
    pub budget: Option<RunBudget>,
    pub budget_state: RunBudgetState,
    pub consumption: RunConsumption,
    pub route_decisions: Vec<AgentRouteDecisionView>,
    pub attempts: Vec<AgentAttemptView>,
    pub thread_ids: Vec<String>,
    pub tool_calls: u64,
    pub artifacts: Vec<AgentArtifactView>,
    pub verifications: Vec<AgentVerificationView>,
    pub final_artifact_ids: Vec<String>,
    pub failure: Option<FailureInfo>,
    pub recovery: RunRecovery,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRunReport {
    pub ok: bool,
    pub schema_version: &'static str,
    pub store_path: String,
    pub as_of_timestamp_ms: Option<u64>,
    pub tasks: Vec<AgentTaskView>,
    pub runs: Vec<AgentRunView>,
    pub safety: SafetySummary,
}

impl AgentRunReport {
    pub fn run(&self, run_id: &str) -> Result<&AgentRunView> {
        self.runs
            .iter()
            .find(|run| run.run_id == run_id)
            .with_context(|| format!("runId '{run_id}' was not found"))
    }

    pub fn task(&self, task_id: &str) -> Result<&AgentTaskView> {
        self.tasks
            .iter()
            .find(|task| task.task_id == task_id)
            .with_context(|| format!("taskId '{task_id}' was not found"))
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRunMutationReport {
    pub ok: bool,
    pub schema_version: &'static str,
    pub action: &'static str,
    pub event_id: String,
    pub task_id: String,
    pub run_id: Option<String>,
    pub attempt_id: Option<String>,
    pub task: AgentTaskView,
    pub run: Option<AgentRunView>,
    pub safety: SafetySummary,
}

#[derive(Debug, Clone)]
pub struct AgentRunStore {
    events: ObservabilityStore,
}

impl AgentRunStore {
    pub fn from_environment(override_path: Option<PathBuf>) -> Result<Self> {
        Ok(Self {
            events: ObservabilityStore::from_environment(override_path)?,
        })
    }

    pub fn new(path: PathBuf) -> Self {
        Self {
            events: ObservabilityStore::new(path),
        }
    }

    pub fn path(&self) -> &Path {
        self.events.path()
    }

    pub fn report(&self) -> Result<AgentRunReport> {
        project_agent_runs(&self.events.load_verified()?, self.events.path())
    }

    pub fn apply(&self, action: AgentRunAction) -> Result<AgentRunMutationReport> {
        let event_id = generate_id("evt");
        let event_id_for_build = event_id.clone();
        let action_for_build = action.clone();
        self.events.append_checked(move |events| {
            let report = project_agent_runs(events, Path::new(""))?;
            let event = build_action_event(
                &report,
                &action_for_build,
                event_id_for_build,
                now_ms()?,
                events,
            )?;
            Ok(vec![event])
        })?;
        self.mutation_report(action, event_id)
    }

    pub fn decide_route(
        &self,
        run_id: &str,
        policy: &RoutePolicy,
        request: &RouteRequest,
        homes: &[RouteHomeState],
    ) -> Result<(RouteDecision, AgentRunMutationReport)> {
        let engine = RouteEngine::new(policy)?;
        let event_id = generate_id("evt");
        let event_id_for_build = event_id.clone();
        let run_id_for_build = run_id.to_owned();
        let decision_slot = RefCell::new(None);
        self.events.append_checked(|events| {
            let report = project_agent_runs(events, Path::new(""))?;
            require_active_run(&report, &run_id_for_build)?;
            let timestamp_ms = now_ms()?;
            let decision = engine.evaluate(request, events, homes, timestamp_ms)?;
            let action = AgentRunAction::RecordRouteDecision {
                run_id: run_id_for_build.clone(),
                decision: Box::new(decision.clone()),
            };
            let event =
                build_action_event(&report, &action, event_id_for_build, timestamp_ms, events)?;
            decision_slot.replace(Some(decision));
            Ok(vec![event])
        })?;
        let decision = decision_slot
            .into_inner()
            .context("route decision was not produced")?;
        let mutation = self.mutation_report(
            AgentRunAction::RecordRouteDecision {
                run_id: run_id.to_owned(),
                decision: Box::new(decision.clone()),
            },
            event_id,
        )?;
        Ok((decision, mutation))
    }

    fn mutation_report(
        &self,
        action: AgentRunAction,
        event_id: String,
    ) -> Result<AgentRunMutationReport> {
        let report = self.report()?;
        let (task_id, run_id, attempt_id) = action.task_run_attempt(&report)?;
        let task = report.task(&task_id)?.clone();
        let run = run_id
            .as_deref()
            .map(|id| report.run(id).cloned())
            .transpose()?;
        Ok(AgentRunMutationReport {
            ok: true,
            schema_version: AGENT_RUN_MUTATION_SCHEMA_VERSION,
            action: action.name(),
            event_id,
            task_id,
            run_id,
            attempt_id,
            task,
            run,
            safety: agent_run_safety(false),
        })
    }
}

pub fn generate_id(prefix: &str) -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis());
    let sequence = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    format!(
        "{prefix}-{timestamp:x}-{:x}-{sequence:x}",
        std::process::id()
    )
}

fn now_ms() -> Result<u64> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before UNIX epoch")?
        .as_millis();
    u64::try_from(millis).context("timestamp does not fit in u64")
}

fn build_action_event(
    report: &AgentRunReport,
    action: &AgentRunAction,
    event_id: String,
    timestamp_ms: u64,
    events: &[ObservabilityEvent],
) -> Result<ObservabilityEvent> {
    match action {
        AgentRunAction::CreateTask {
            task_id,
            label,
            kind,
        } => {
            if report.tasks.iter().any(|task| task.task_id == *task_id) {
                bail!("taskId '{task_id}' already exists");
            }
            let mut event = base_event(
                event_id,
                timestamp_ms,
                ObservabilityEventType::TaskCreated,
                ObservabilityStatus::Started,
                task_id,
                None,
                None,
            );
            event.details.task = Some(TaskDescriptor {
                label: label.clone(),
                kind: kind.clone(),
            });
            Ok(event)
        }
        AgentRunAction::StartRun {
            task_id,
            run_id,
            budget,
        } => {
            report.task(task_id)?;
            if report.runs.iter().any(|run| run.run_id == *run_id) {
                bail!("runId '{run_id}' already exists");
            }
            let mut event = base_event(
                event_id,
                timestamp_ms,
                ObservabilityEventType::RunStarted,
                ObservabilityStatus::Started,
                task_id,
                Some(run_id),
                None,
            );
            event.details.budget = budget.clone();
            Ok(event)
        }
        AgentRunAction::StartAttempt {
            run_id,
            attempt_id,
            identity,
            transition,
            route_reason,
            route_decision_id,
        } => {
            let run = require_active_run(report, run_id)?;
            ensure_budget_allows_attempt(run)?;
            if report
                .runs
                .iter()
                .flat_map(|candidate| &candidate.attempts)
                .any(|attempt| attempt.attempt_id == *attempt_id)
            {
                bail!("attemptId '{attempt_id}' already exists");
            }
            require_identity(identity)?;
            if matches!(
                transition.kind,
                AttemptTransitionKind::Retry | AttemptTransitionKind::Migration
            ) && route_reason
                .as_deref()
                .is_none_or(|reason| reason.trim().is_empty())
            {
                bail!("retry and migration attempts require a route reason");
            }
            if transition.kind == AttemptTransitionKind::Migration {
                let source_id = transition
                    .from_attempt_id
                    .as_deref()
                    .context("migration requires fromAttemptId")?;
                let source = run
                    .attempts
                    .iter()
                    .find(|attempt| attempt.attempt_id == source_id)
                    .with_context(|| format!("fromAttemptId '{source_id}' was not found"))?;
                if source.identity.home_id == identity.home_id
                    && source.identity.account_id == identity.account_id
                    && source.identity.model == identity.model
                {
                    bail!("migration must change Home, account, or model");
                }
            }
            let mut event = base_event(
                event_id,
                timestamp_ms,
                ObservabilityEventType::AttemptStarted,
                ObservabilityStatus::Running,
                &run.task_id,
                Some(run_id),
                Some(attempt_id),
            );
            event.identity = identity.clone();
            event.details.transition = Some(transition.clone());
            event.details.route_reason = route_reason.clone();
            event.details.route_decision_id = route_decision_id.clone();
            if let Some(source) = &transition.from_attempt_id {
                event.parent_event_id = terminal_event_id(events, source);
            }
            Ok(event)
        }
        AgentRunAction::RecordRouteDecision { run_id, decision } => {
            let run = require_active_run(report, run_id)?;
            let selected = decision.selected.as_ref();
            let mut event = base_event(
                event_id,
                timestamp_ms,
                ObservabilityEventType::RouteDecided,
                if selected.is_some() {
                    ObservabilityStatus::Succeeded
                } else {
                    ObservabilityStatus::Failed
                },
                &run.task_id,
                Some(run_id),
                None,
            );
            if let Some(selection) = selected {
                event.identity = selection.identity.clone();
            } else {
                event.failure = Some(FailureInfo {
                    code: "no_eligible_route".to_owned(),
                    phase: Some("routing".to_owned()),
                    reason: bounded_event_text(&decision.decision_reason, 2_048),
                    retryable: true,
                });
            }
            let rejection_summary = decision
                .candidates
                .iter()
                .filter(|candidate| !candidate.rejection_reasons.is_empty())
                .take(64)
                .map(|candidate| {
                    bounded_event_text(
                        &format!(
                            "{}: {}",
                            candidate.candidate_id,
                            candidate.rejection_reasons.join(" | ")
                        ),
                        2_048,
                    )
                })
                .collect::<Vec<_>>();
            event.details.routing = Some(RouteDecisionDetails {
                request_id: decision.request_id.clone(),
                policy_id: decision.policy_id.clone(),
                policy_revision: decision.policy_revision,
                evaluated_at_timestamp_ms: decision.evaluated_at_timestamp_ms,
                observed_event_count: u64::try_from(decision.observed_event_count)
                    .context("observed route event count exceeds u64")?,
                request: decision.request.clone(),
                selected_candidate_id: selected.map(|value| value.candidate_id.clone()),
                selected_score_basis_points: selected.map(|value| value.total_score_basis_points),
                estimated_cost_microusd: selected.map(|value| value.estimated_cost_microusd),
                evaluated_candidates: u32::try_from(decision.evaluated_candidates)
                    .context("evaluated route candidate count exceeds u32")?,
                eligible_candidates: u32::try_from(decision.eligible_candidates)
                    .context("eligible route candidate count exceeds u32")?,
                home_locked: decision.locks.home.is_some(),
                model_locked: decision.locks.model.is_some(),
                sensitive_directory: decision.locks.sensitive_directory,
                reasons: selected.map_or_else(Vec::new, |value| value.reasons.clone()),
                rejection_summary,
                candidates: decision
                    .candidates
                    .iter()
                    .map(|candidate| RouteCandidateDecisionSnapshot {
                        candidate_id: candidate.candidate_id.clone(),
                        identity: candidate.identity.clone(),
                        eligible: candidate.eligible,
                        total_score_basis_points: candidate.total_score_basis_points,
                        estimated_cost_microusd: candidate.estimated_cost_microusd,
                        historical_attempts: candidate.historical_attempts,
                        historical_success_basis_points: candidate.historical_success_basis_points,
                        active_attempts: candidate.active_attempts,
                        quota_remaining_basis_points: candidate.quota_remaining_basis_points,
                        components: candidate
                            .components
                            .iter()
                            .map(|component| RouteScoreSnapshot {
                                name: component.name.clone(),
                                score_basis_points: component.score_basis_points,
                                weight_basis_points: component.weight_basis_points,
                                weighted_points: component.weighted_points,
                                reason: component.reason.clone(),
                            })
                            .collect(),
                        reasons: candidate.reasons.clone(),
                        rejection_reasons: candidate.rejection_reasons.clone(),
                    })
                    .collect(),
            });
            Ok(event)
        }
        AgentRunAction::CompleteAttempt {
            run_id,
            attempt_id,
            thread_id,
            usage,
        } => terminal_attempt_event(
            report,
            event_id,
            timestamp_ms,
            run_id,
            attempt_id,
            TerminalAttemptSpec {
                event_type: ObservabilityEventType::AttemptCompleted,
                status: ObservabilityStatus::Succeeded,
                thread_id: thread_id.clone(),
                usage: usage.clone(),
                failure: None,
            },
        ),
        AgentRunAction::FailAttempt {
            run_id,
            attempt_id,
            thread_id,
            status,
            usage,
            failure,
        } => {
            require_failure_status(*status)?;
            terminal_attempt_event(
                report,
                event_id,
                timestamp_ms,
                run_id,
                attempt_id,
                TerminalAttemptSpec {
                    event_type: ObservabilityEventType::AttemptFailed,
                    status: *status,
                    thread_id: thread_id.clone(),
                    usage: usage.clone(),
                    failure: Some(failure.clone()),
                },
            )
        }
        AgentRunAction::LinkThread {
            run_id,
            attempt_id,
            thread_id,
            parent_thread_id,
            fork_id,
        } => {
            let run = require_active_run(report, run_id)?;
            if report
                .runs
                .iter()
                .flat_map(|candidate| &candidate.thread_ids)
                .any(|existing| existing == thread_id)
            {
                bail!("threadId '{thread_id}' is already linked");
            }
            if let Some(attempt_id) = attempt_id {
                require_attempt(run, attempt_id)?;
            }
            let mut event = base_event(
                event_id,
                timestamp_ms,
                ObservabilityEventType::ThreadLinked,
                ObservabilityStatus::Running,
                &run.task_id,
                Some(run_id),
                attempt_id.as_deref(),
            );
            event.trace.thread_id = Some(thread_id.clone());
            event.trace.parent_thread_id = parent_thread_id.clone();
            event.trace.fork_id = fork_id.clone();
            Ok(event)
        }
        AgentRunAction::RecordTool {
            run_id,
            attempt_id,
            thread_id,
            tool_name,
            usage,
        } => {
            let (run, attempt) = require_active_attempt(report, run_id, attempt_id)?;
            require_thread(run, thread_id.as_deref())?;
            let mut event = attempt_event(
                event_id,
                timestamp_ms,
                ObservabilityEventType::ToolCallCompleted,
                ObservabilityStatus::Succeeded,
                run,
                attempt,
                thread_id.clone(),
            );
            event.tool_name = Some(tool_name.clone());
            event.usage = usage.clone();
            Ok(event)
        }
        AgentRunAction::RecordArtifact {
            run_id,
            attempt_id,
            thread_id,
            artifact_id,
            artifact_kind,
        } => {
            let (run, attempt) = require_active_attempt(report, run_id, attempt_id)?;
            require_thread(run, thread_id.as_deref())?;
            if report
                .runs
                .iter()
                .flat_map(|candidate| &candidate.artifacts)
                .any(|artifact| artifact.artifact_id == *artifact_id)
            {
                bail!("artifactId '{artifact_id}' already exists");
            }
            let mut event = attempt_event(
                event_id,
                timestamp_ms,
                ObservabilityEventType::ArtifactCreated,
                ObservabilityStatus::Succeeded,
                run,
                attempt,
                thread_id.clone(),
            );
            event.artifact_kind = Some(artifact_kind.clone());
            event.details.artifact_id = Some(artifact_id.clone());
            Ok(event)
        }
        AgentRunAction::RecordVerification {
            run_id,
            attempt_id,
            thread_id,
            verification_id,
            verification_kind,
            target_artifact_id,
            succeeded,
            duration_ms,
        } => {
            let (run, attempt) = require_active_attempt(report, run_id, attempt_id)?;
            require_thread(run, thread_id.as_deref())?;
            if report
                .runs
                .iter()
                .flat_map(|candidate| &candidate.verifications)
                .any(|verification| verification.verification_id == *verification_id)
            {
                bail!("verificationId '{verification_id}' already exists");
            }
            if let Some(artifact_id) = target_artifact_id {
                if !run
                    .artifacts
                    .iter()
                    .any(|artifact| artifact.artifact_id == *artifact_id)
                {
                    bail!("targetArtifactId '{artifact_id}' was not found in run '{run_id}'");
                }
            }
            let mut event = attempt_event(
                event_id,
                timestamp_ms,
                ObservabilityEventType::VerificationCompleted,
                if *succeeded {
                    ObservabilityStatus::Succeeded
                } else {
                    ObservabilityStatus::Failed
                },
                run,
                attempt,
                thread_id.clone(),
            );
            event.verification_kind = Some(verification_kind.clone());
            event.details.verification_id = Some(verification_id.clone());
            event.details.target_artifact_id = target_artifact_id.clone();
            event.usage.duration_ms = *duration_ms;
            Ok(event)
        }
        AgentRunAction::CompleteRun {
            run_id,
            final_artifact_ids,
        } => {
            let run = require_active_run(report, run_id)?;
            ensure_no_active_attempts(run)?;
            if !run
                .attempts
                .iter()
                .any(|attempt| attempt.status == AttemptLifecycleStatus::Succeeded)
            {
                bail!("runId '{run_id}' cannot complete without a successful attempt");
            }
            for artifact in final_artifact_ids {
                if !run
                    .artifacts
                    .iter()
                    .any(|candidate| candidate.artifact_id == *artifact)
                {
                    bail!("final artifactId '{artifact}' was not found in run '{run_id}'");
                }
            }
            let mut event = base_event(
                event_id,
                timestamp_ms,
                ObservabilityEventType::RunCompleted,
                ObservabilityStatus::Succeeded,
                &run.task_id,
                Some(run_id),
                None,
            );
            event.details.final_artifact_ids = final_artifact_ids.clone();
            Ok(event)
        }
        AgentRunAction::FailRun {
            run_id,
            status,
            failure,
        } => {
            require_failure_status(*status)?;
            let run = require_active_run(report, run_id)?;
            ensure_no_active_attempts(run)?;
            let mut event = base_event(
                event_id,
                timestamp_ms,
                ObservabilityEventType::RunFailed,
                *status,
                &run.task_id,
                Some(run_id),
                None,
            );
            event.failure = Some(failure.clone());
            Ok(event)
        }
    }
}

struct TerminalAttemptSpec {
    event_type: ObservabilityEventType,
    status: ObservabilityStatus,
    thread_id: Option<String>,
    usage: UsageDelta,
    failure: Option<FailureInfo>,
}

fn terminal_attempt_event(
    report: &AgentRunReport,
    event_id: String,
    timestamp_ms: u64,
    run_id: &str,
    attempt_id: &str,
    spec: TerminalAttemptSpec,
) -> Result<ObservabilityEvent> {
    let (run, attempt) = require_active_attempt(report, run_id, attempt_id)?;
    require_thread(run, spec.thread_id.as_deref())?;
    let mut event = attempt_event(
        event_id,
        timestamp_ms,
        spec.event_type,
        spec.status,
        run,
        attempt,
        spec.thread_id,
    );
    event.usage = spec.usage;
    event.failure = spec.failure;
    Ok(event)
}

fn attempt_event(
    event_id: String,
    timestamp_ms: u64,
    event_type: ObservabilityEventType,
    status: ObservabilityStatus,
    run: &AgentRunView,
    attempt: &AgentAttemptView,
    thread_id: Option<String>,
) -> ObservabilityEvent {
    let mut event = base_event(
        event_id,
        timestamp_ms,
        event_type,
        status,
        &run.task_id,
        Some(&run.run_id),
        Some(&attempt.attempt_id),
    );
    event.trace.thread_id = thread_id;
    event.identity = attempt.identity.clone();
    event
}

fn base_event(
    event_id: String,
    timestamp_ms: u64,
    event_type: ObservabilityEventType,
    status: ObservabilityStatus,
    task_id: &str,
    run_id: Option<&str>,
    attempt_id: Option<&str>,
) -> ObservabilityEvent {
    ObservabilityEvent {
        schema_version: OBSERVABILITY_EVENT_SCHEMA_VERSION.to_owned(),
        event_id,
        timestamp_ms,
        event_type,
        status,
        trace: TraceContext {
            task_id: Some(task_id.to_owned()),
            run_id: run_id.map(str::to_owned),
            attempt_id: attempt_id.map(str::to_owned),
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

fn project_agent_runs(events: &[ObservabilityEvent], path: &Path) -> Result<AgentRunReport> {
    let mut tasks = BTreeMap::<String, AgentTaskView>::new();
    let mut runs = BTreeMap::<String, AgentRunView>::new();
    let mut as_of = None;
    for event in events {
        as_of = Some(as_of.map_or(event.timestamp_ms, |value: u64| {
            value.max(event.timestamp_ms)
        }));
        let task_id = event.trace.task_id.as_deref();
        let run_id = event.trace.run_id.as_deref();
        match event.event_type {
            ObservabilityEventType::TaskCreated => {
                let task_id = task_id.context("task_created is missing taskId")?;
                let descriptor = event.details.task.clone().unwrap_or(TaskDescriptor {
                    label: task_id.to_owned(),
                    kind: None,
                });
                tasks.insert(
                    task_id.to_owned(),
                    AgentTaskView {
                        task_id: task_id.to_owned(),
                        label: descriptor.label,
                        kind: descriptor.kind,
                        created_at_ms: event.timestamp_ms,
                        status: TaskLifecycleStatus::Pending,
                        run_ids: Vec::new(),
                    },
                );
            }
            ObservabilityEventType::RunStarted => {
                let task_id = task_id.context("run_started is missing taskId")?;
                let run_id = run_id.context("run_started is missing runId")?;
                let task = tasks.get_mut(task_id).with_context(|| {
                    format!("run '{run_id}' references unknown task '{task_id}'")
                })?;
                task.run_ids.push(run_id.to_owned());
                runs.insert(
                    run_id.to_owned(),
                    AgentRunView {
                        run_id: run_id.to_owned(),
                        task_id: task_id.to_owned(),
                        status: RunLifecycleStatus::Running,
                        started_at_ms: event.timestamp_ms,
                        finished_at_ms: None,
                        budget: event.details.budget.clone(),
                        budget_state: RunBudgetState::default(),
                        consumption: RunConsumption::default(),
                        route_decisions: Vec::new(),
                        attempts: Vec::new(),
                        thread_ids: Vec::new(),
                        tool_calls: 0,
                        artifacts: Vec::new(),
                        verifications: Vec::new(),
                        final_artifact_ids: Vec::new(),
                        failure: None,
                        recovery: default_recovery(),
                    },
                );
            }
            ObservabilityEventType::RouteDecided => {
                let run = run_mut(&mut runs, run_id, event)?;
                let routing = event
                    .details
                    .routing
                    .as_ref()
                    .context("route_decided is missing routing details")?;
                run.route_decisions.push(AgentRouteDecisionView {
                    event_id: event.event_id.clone(),
                    request_id: routing.request_id.clone(),
                    policy_id: routing.policy_id.clone(),
                    policy_revision: routing.policy_revision,
                    evaluated_at_timestamp_ms: routing.evaluated_at_timestamp_ms,
                    observed_event_count: routing.observed_event_count,
                    request: routing.request.clone(),
                    decided_at_ms: event.timestamp_ms,
                    succeeded: event.status == ObservabilityStatus::Succeeded,
                    identity: event.identity.clone(),
                    selected_candidate_id: routing.selected_candidate_id.clone(),
                    selected_score_basis_points: routing.selected_score_basis_points,
                    estimated_cost_microusd: routing.estimated_cost_microusd,
                    evaluated_candidates: routing.evaluated_candidates,
                    eligible_candidates: routing.eligible_candidates,
                    home_locked: routing.home_locked,
                    model_locked: routing.model_locked,
                    sensitive_directory: routing.sensitive_directory,
                    reasons: routing.reasons.clone(),
                    rejection_summary: routing.rejection_summary.clone(),
                    candidates: routing.candidates.clone(),
                });
            }
            ObservabilityEventType::AttemptStarted => {
                let run = run_mut(&mut runs, run_id, event)?;
                let attempt_id = event
                    .trace
                    .attempt_id
                    .as_deref()
                    .context("attempt_started is missing attemptId")?;
                run.attempts.push(AgentAttemptView {
                    attempt_id: attempt_id.to_owned(),
                    ordinal: run.attempts.len() + 1,
                    status: AttemptLifecycleStatus::Running,
                    started_at_ms: event.timestamp_ms,
                    finished_at_ms: None,
                    identity: event.identity.clone(),
                    transition: event
                        .details
                        .transition
                        .clone()
                        .unwrap_or(AttemptTransition {
                            kind: AttemptTransitionKind::Initial,
                            from_attempt_id: None,
                        }),
                    route_reason: event.details.route_reason.clone(),
                    route_decision_id: event.details.route_decision_id.clone(),
                    thread_ids: Vec::new(),
                    usage: UsageDelta::default(),
                    failure: None,
                });
            }
            ObservabilityEventType::ThreadLinked => {
                let run = run_mut(&mut runs, run_id, event)?;
                let thread_id = event
                    .trace
                    .thread_id
                    .as_deref()
                    .context("thread_linked is missing threadId")?;
                run.thread_ids.push(thread_id.to_owned());
                if let Some(attempt_id) = &event.trace.attempt_id {
                    attempt_mut(run, attempt_id)?
                        .thread_ids
                        .push(thread_id.to_owned());
                }
            }
            ObservabilityEventType::ToolCallCompleted => {
                run_mut(&mut runs, run_id, event)?.tool_calls += 1;
            }
            ObservabilityEventType::ArtifactCreated => {
                let run = run_mut(&mut runs, run_id, event)?;
                run.artifacts.push(AgentArtifactView {
                    artifact_id: event
                        .details
                        .artifact_id
                        .clone()
                        .unwrap_or_else(|| event.event_id.clone()),
                    artifact_kind: event
                        .artifact_kind
                        .clone()
                        .unwrap_or_else(|| "unknown".to_owned()),
                    attempt_id: event
                        .trace
                        .attempt_id
                        .clone()
                        .context("artifact_created is missing attemptId")?,
                    thread_id: event.trace.thread_id.clone(),
                    created_at_ms: event.timestamp_ms,
                });
            }
            ObservabilityEventType::VerificationCompleted => {
                let run = run_mut(&mut runs, run_id, event)?;
                run.verifications.push(AgentVerificationView {
                    verification_id: event
                        .details
                        .verification_id
                        .clone()
                        .unwrap_or_else(|| event.event_id.clone()),
                    verification_kind: event
                        .verification_kind
                        .clone()
                        .unwrap_or_else(|| "unknown".to_owned()),
                    attempt_id: event
                        .trace
                        .attempt_id
                        .clone()
                        .context("verification_completed is missing attemptId")?,
                    thread_id: event.trace.thread_id.clone(),
                    target_artifact_id: event.details.target_artifact_id.clone(),
                    succeeded: event.status == ObservabilityStatus::Succeeded,
                    duration_ms: event.usage.duration_ms,
                    completed_at_ms: event.timestamp_ms,
                });
            }
            ObservabilityEventType::AttemptCompleted | ObservabilityEventType::AttemptFailed => {
                let run = run_mut(&mut runs, run_id, event)?;
                let attempt_id = event
                    .trace
                    .attempt_id
                    .as_deref()
                    .context("terminal attempt event is missing attemptId")?;
                let status = attempt_status(event.status)?;
                let attempt = attempt_mut(run, attempt_id)?;
                attempt.status = status;
                attempt.finished_at_ms = Some(event.timestamp_ms);
                attempt.usage = event.usage.clone();
                attempt.failure = event.failure.clone();
                add_consumption(&mut run.consumption, &event.usage, status)?;
            }
            ObservabilityEventType::RunCompleted | ObservabilityEventType::RunFailed => {
                let run = run_mut(&mut runs, run_id, event)?;
                run.status = run_status(event.status)?;
                run.finished_at_ms = Some(event.timestamp_ms);
                run.final_artifact_ids = event.details.final_artifact_ids.clone();
                run.failure = event.failure.clone();
            }
            ObservabilityEventType::QuotaSnapshot
            | ObservabilityEventType::RateLimited
            | ObservabilityEventType::AuthInvalid
            | ObservabilityEventType::HomeHealth => {}
        }
    }

    for run in runs.values_mut() {
        run.consumption.attempts = run.attempts.len();
        run.budget_state = budget_state(run.budget.as_ref(), &run.consumption);
        run.recovery = recovery(run);
    }
    for task in tasks.values_mut() {
        let owned = task
            .run_ids
            .iter()
            .filter_map(|run_id| runs.get(run_id))
            .collect::<Vec<_>>();
        task.status = if owned.is_empty() {
            TaskLifecycleStatus::Pending
        } else if owned
            .iter()
            .any(|run| run.status == RunLifecycleStatus::Running)
        {
            TaskLifecycleStatus::Running
        } else if owned
            .iter()
            .any(|run| run.status == RunLifecycleStatus::Succeeded)
        {
            TaskLifecycleStatus::Succeeded
        } else {
            TaskLifecycleStatus::Failed
        };
    }
    Ok(AgentRunReport {
        ok: true,
        schema_version: AGENT_RUN_REPORT_SCHEMA_VERSION,
        store_path: path.to_string_lossy().into_owned(),
        as_of_timestamp_ms: as_of,
        tasks: tasks.into_values().collect(),
        runs: runs.into_values().collect(),
        safety: agent_run_safety(true),
    })
}

fn add_consumption(
    target: &mut RunConsumption,
    usage: &UsageDelta,
    status: AttemptLifecycleStatus,
) -> Result<()> {
    target.input_tokens = checked(target.input_tokens, usage.input_tokens, "input tokens")?;
    target.output_tokens = checked(target.output_tokens, usage.output_tokens, "output tokens")?;
    target.total_tokens = checked(
        target.total_tokens,
        usage
            .input_tokens
            .checked_add(usage.output_tokens)
            .context("attempt token count overflow")?,
        "total tokens",
    )?;
    target.cached_input_tokens = checked(
        target.cached_input_tokens,
        usage.cached_input_tokens,
        "cached input tokens",
    )?;
    target.duration_ms = checked(target.duration_ms, usage.duration_ms, "duration")?;
    target.estimated_cost_microusd = checked(
        target.estimated_cost_microusd,
        usage.estimated_cost_microusd,
        "estimated cost",
    )?;
    target.retries = checked(target.retries, usage.retries, "retry count")?;
    if matches!(
        status,
        AttemptLifecycleStatus::Failed
            | AttemptLifecycleStatus::Cancelled
            | AttemptLifecycleStatus::TimedOut
    ) {
        let tokens = usage
            .input_tokens
            .checked_add(usage.output_tokens)
            .context("failed attempt token count overflow")?;
        target.failed_attempt_tokens = checked(
            target.failed_attempt_tokens,
            tokens,
            "failed attempt tokens",
        )?;
        target.failed_attempt_duration_ms = checked(
            target.failed_attempt_duration_ms,
            usage.duration_ms,
            "failed attempt duration",
        )?;
        target.failed_attempt_cost_microusd = checked(
            target.failed_attempt_cost_microusd,
            usage.estimated_cost_microusd,
            "failed attempt cost",
        )?;
    }
    Ok(())
}

fn budget_state(budget: Option<&RunBudget>, usage: &RunConsumption) -> RunBudgetState {
    let Some(budget) = budget else {
        return RunBudgetState {
            within_budget: true,
            exhausted_dimensions: Vec::new(),
        };
    };
    let mut exhausted = Vec::new();
    if budget
        .max_total_tokens
        .is_some_and(|limit| usage.total_tokens >= limit)
    {
        exhausted.push("tokens".to_owned());
    }
    if budget
        .max_duration_ms
        .is_some_and(|limit| usage.duration_ms >= limit)
    {
        exhausted.push("duration".to_owned());
    }
    if budget
        .max_cost_microusd
        .is_some_and(|limit| usage.estimated_cost_microusd >= limit)
    {
        exhausted.push("cost".to_owned());
    }
    if budget
        .max_attempts
        .is_some_and(|limit| usage.attempts >= limit as usize)
    {
        exhausted.push("attempts".to_owned());
    }
    RunBudgetState {
        within_budget: exhausted.is_empty(),
        exhausted_dimensions: exhausted,
    }
}

fn recovery(run: &AgentRunView) -> RunRecovery {
    if run.status != RunLifecycleStatus::Running {
        return RunRecovery {
            can_retry: false,
            can_migrate: false,
            recommended_action: "none".to_owned(),
            reason: "run is terminal".to_owned(),
        };
    }
    if !run.budget_state.within_budget {
        return RunRecovery {
            can_retry: false,
            can_migrate: false,
            recommended_action: "pause".to_owned(),
            reason: format!(
                "budget exhausted: {}",
                run.budget_state.exhausted_dimensions.join(",")
            ),
        };
    }
    if run
        .attempts
        .iter()
        .any(|attempt| attempt.status == AttemptLifecycleStatus::Running)
    {
        return RunRecovery {
            can_retry: false,
            can_migrate: false,
            recommended_action: "wait".to_owned(),
            reason: "an attempt is still running".to_owned(),
        };
    }
    let Some(last) = run.attempts.last() else {
        return RunRecovery {
            can_retry: false,
            can_migrate: false,
            recommended_action: "start_attempt".to_owned(),
            reason: "run has no attempts".to_owned(),
        };
    };
    if last.status == AttemptLifecycleStatus::Succeeded {
        return RunRecovery {
            can_retry: false,
            can_migrate: false,
            recommended_action: "complete".to_owned(),
            reason: "latest attempt succeeded".to_owned(),
        };
    }
    let retryable = last
        .failure
        .as_ref()
        .is_some_and(|failure| failure.retryable);
    RunRecovery {
        can_retry: retryable,
        can_migrate: true,
        recommended_action: if retryable { "retry" } else { "migrate" }.to_owned(),
        reason: if retryable {
            "latest failure is retryable and budget remains"
        } else {
            "latest failure is terminal for this identity; migrate Home or model"
        }
        .to_owned(),
    }
}

fn default_recovery() -> RunRecovery {
    RunRecovery {
        can_retry: false,
        can_migrate: false,
        recommended_action: "start_attempt".to_owned(),
        reason: "run has no attempts".to_owned(),
    }
}

fn require_active_run<'a>(report: &'a AgentRunReport, run_id: &str) -> Result<&'a AgentRunView> {
    let run = report.run(run_id)?;
    if run.status != RunLifecycleStatus::Running {
        bail!("runId '{run_id}' is already terminal");
    }
    Ok(run)
}

fn require_attempt<'a>(run: &'a AgentRunView, attempt_id: &str) -> Result<&'a AgentAttemptView> {
    run.attempts
        .iter()
        .find(|attempt| attempt.attempt_id == attempt_id)
        .with_context(|| {
            format!(
                "attemptId '{attempt_id}' was not found in run '{}'",
                run.run_id
            )
        })
}

fn require_active_attempt<'a>(
    report: &'a AgentRunReport,
    run_id: &str,
    attempt_id: &str,
) -> Result<(&'a AgentRunView, &'a AgentAttemptView)> {
    let run = require_active_run(report, run_id)?;
    let attempt = require_attempt(run, attempt_id)?;
    if attempt.status != AttemptLifecycleStatus::Running {
        bail!("attemptId '{attempt_id}' is already terminal");
    }
    Ok((run, attempt))
}

fn require_thread(run: &AgentRunView, thread_id: Option<&str>) -> Result<()> {
    if let Some(thread_id) = thread_id {
        if !run
            .thread_ids
            .iter()
            .any(|candidate| candidate == thread_id)
        {
            bail!(
                "threadId '{thread_id}' is not linked to run '{}'",
                run.run_id
            );
        }
    }
    Ok(())
}

fn require_identity(identity: &ExecutionIdentity) -> Result<()> {
    if identity
        .home_id
        .as_deref()
        .is_none_or(|value| value.trim().is_empty())
    {
        bail!("attempt identity requires homeId");
    }
    if identity
        .model
        .as_deref()
        .is_none_or(|value| value.trim().is_empty())
    {
        bail!("attempt identity requires model");
    }
    Ok(())
}

fn ensure_no_active_attempts(run: &AgentRunView) -> Result<()> {
    if run
        .attempts
        .iter()
        .any(|attempt| attempt.status == AttemptLifecycleStatus::Running)
    {
        bail!("runId '{}' has active attempts", run.run_id);
    }
    Ok(())
}

fn ensure_budget_allows_attempt(run: &AgentRunView) -> Result<()> {
    if !run.budget_state.within_budget {
        bail!(
            "runId '{}' budget is exhausted: {}",
            run.run_id,
            run.budget_state.exhausted_dimensions.join(",")
        );
    }
    Ok(())
}

fn require_failure_status(status: ObservabilityStatus) -> Result<()> {
    if !matches!(
        status,
        ObservabilityStatus::Failed
            | ObservabilityStatus::Cancelled
            | ObservabilityStatus::TimedOut
    ) {
        bail!("failure status must be failed, cancelled, or timed_out");
    }
    Ok(())
}

fn attempt_status(status: ObservabilityStatus) -> Result<AttemptLifecycleStatus> {
    match status {
        ObservabilityStatus::Succeeded => Ok(AttemptLifecycleStatus::Succeeded),
        ObservabilityStatus::Failed => Ok(AttemptLifecycleStatus::Failed),
        ObservabilityStatus::Cancelled => Ok(AttemptLifecycleStatus::Cancelled),
        ObservabilityStatus::TimedOut => Ok(AttemptLifecycleStatus::TimedOut),
        _ => bail!("attempt terminal event has non-terminal status"),
    }
}

fn run_status(status: ObservabilityStatus) -> Result<RunLifecycleStatus> {
    match status {
        ObservabilityStatus::Succeeded => Ok(RunLifecycleStatus::Succeeded),
        ObservabilityStatus::Failed => Ok(RunLifecycleStatus::Failed),
        ObservabilityStatus::Cancelled => Ok(RunLifecycleStatus::Cancelled),
        ObservabilityStatus::TimedOut => Ok(RunLifecycleStatus::TimedOut),
        _ => bail!("run terminal event has non-terminal status"),
    }
}

fn run_mut<'a>(
    runs: &'a mut BTreeMap<String, AgentRunView>,
    run_id: Option<&str>,
    event: &ObservabilityEvent,
) -> Result<&'a mut AgentRunView> {
    let run_id = run_id.with_context(|| format!("event {} is missing runId", event.event_id))?;
    runs.get_mut(run_id).with_context(|| {
        format!(
            "event {} references unknown runId '{run_id}'",
            event.event_id
        )
    })
}

fn attempt_mut<'a>(
    run: &'a mut AgentRunView,
    attempt_id: &str,
) -> Result<&'a mut AgentAttemptView> {
    run.attempts
        .iter_mut()
        .find(|attempt| attempt.attempt_id == attempt_id)
        .with_context(|| {
            format!(
                "attemptId '{attempt_id}' was not found in run '{}'",
                run.run_id
            )
        })
}

fn terminal_event_id(events: &[ObservabilityEvent], attempt_id: &str) -> Option<String> {
    events
        .iter()
        .rev()
        .find(|event| {
            event.trace.attempt_id.as_deref() == Some(attempt_id)
                && matches!(
                    event.event_type,
                    ObservabilityEventType::AttemptCompleted
                        | ObservabilityEventType::AttemptFailed
                )
        })
        .map(|event| event.event_id.clone())
}

fn checked(left: u64, right: u64, label: &str) -> Result<u64> {
    left.checked_add(right)
        .with_context(|| format!("{label} overflow"))
}

fn bounded_event_text(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_owned();
    }
    let mut end = max_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].trim_end().to_owned()
}

fn agent_run_safety(includes_local_paths: bool) -> SafetySummary {
    SafetySummary {
        secrets_redacted: true,
        includes_secrets: false,
        reads_auth_contents: false,
        includes_local_paths,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn identity(home: &str, model: &str) -> ExecutionIdentity {
        ExecutionIdentity {
            home_id: Some(home.to_owned()),
            home_alias: None,
            account_id: Some(format!("account-{home}")),
            provider: Some("openai".to_owned()),
            model: Some(model.to_owned()),
            agent_role: Some("executor".to_owned()),
        }
    }

    fn route_decision(home: &str, model: &str) -> RouteDecision {
        let selected_identity = identity(home, model);
        let component = crate::RouteScoreComponent {
            name: "task_fit".to_owned(),
            score_basis_points: 8_500,
            weight_basis_points: 10_000,
            weighted_points: 8_500,
            reason: "matched coding specialty".to_owned(),
        };
        RouteDecision {
            ok: true,
            schema_version: crate::ROUTE_DECISION_SCHEMA_VERSION,
            request_id: "request-1".to_owned(),
            task_kind: crate::RouteTaskKind::SimpleCode,
            policy_id: "policy-1".to_owned(),
            policy_revision: 1,
            evaluated_at_timestamp_ms: 0,
            observed_event_count: 2,
            request: RouteRequest {
                schema_version: crate::ROUTE_REQUEST_SCHEMA_VERSION.to_owned(),
                request_id: "request-1".to_owned(),
                task_kind: crate::RouteTaskKind::SimpleCode,
                estimated_context_tokens: 2_000,
                estimated_output_tokens: 500,
                required_capabilities: vec!["coding".to_owned()],
                preferred_specialties: vec!["simple_code".to_owned()],
                sensitive_directory: false,
                required_security_domain: None,
                locked_home: Some(home.to_owned()),
                locked_model: None,
                max_estimated_cost_microusd: None,
                allow_degraded: false,
            },
            selected: Some(crate::RouteSelection {
                candidate_id: "candidate-1".to_owned(),
                identity: selected_identity.clone(),
                total_score_basis_points: 8_500,
                estimated_cost_microusd: 25,
                reasons: vec!["matched coding specialty".to_owned()],
            }),
            evaluated_candidates: 1,
            eligible_candidates: 1,
            candidates: vec![crate::RouteCandidateEvaluation {
                candidate_id: "candidate-1".to_owned(),
                identity: selected_identity,
                eligible: true,
                total_score_basis_points: Some(8_500),
                estimated_cost_microusd: 25,
                historical_attempts: 0,
                historical_success_basis_points: 5_000,
                active_attempts: 0,
                quota_remaining_basis_points: None,
                components: vec![component],
                reasons: vec!["matched coding specialty".to_owned()],
                rejection_reasons: Vec::new(),
            }],
            decision_reason: "selected candidate-1".to_owned(),
            locks: crate::RouteLocks {
                home: Some(home.to_owned()),
                model: None,
                sensitive_directory: false,
            },
            safety: agent_run_safety(false),
        }
    }

    fn route_policy() -> RoutePolicy {
        RoutePolicy {
            schema_version: crate::ROUTE_POLICY_SCHEMA_VERSION.to_owned(),
            policy_id: "policy-atomic".to_owned(),
            revision: 3,
            weights: crate::RouteScoreWeights::default(),
            task_rules: Vec::new(),
            candidates: vec![crate::RouteCandidateProfile {
                candidate_id: "candidate-spark".to_owned(),
                enabled: true,
                home_id: "home-spark".to_owned(),
                home_alias: Some("spark".to_owned()),
                account_id: Some("account-spark".to_owned()),
                provider: Some("openai".to_owned()),
                model: "spark-model".to_owned(),
                specialties: vec!["simple_code".to_owned()],
                capabilities: vec!["coding".to_owned()],
                security_domains: Vec::new(),
                max_context_tokens: 32_000,
                model_strength_basis_points: 7_000,
                input_cost_microusd_per_million_tokens: 100,
                output_cost_microusd_per_million_tokens: 200,
                max_concurrent_runs: 1,
                baseline_duration_ms: Some(1_000),
            }],
            history_prior_success_basis_points: 5_000,
            history_prior_attempts: 2,
            unknown_quota_basis_points: 5_000,
            unknown_health_basis_points: 5_000,
        }
    }

    fn setup(store: &AgentRunStore) -> (String, String, String) {
        let task_id = "task-1".to_owned();
        let run_id = "run-1".to_owned();
        let attempt_id = "attempt-1".to_owned();
        store
            .apply(AgentRunAction::CreateTask {
                task_id: task_id.clone(),
                label: "Compile benchmark".to_owned(),
                kind: Some("coding".to_owned()),
            })
            .expect("create task");
        store
            .apply(AgentRunAction::StartRun {
                task_id: task_id.clone(),
                run_id: run_id.clone(),
                budget: Some(RunBudget {
                    max_total_tokens: Some(1_000),
                    max_duration_ms: Some(60_000),
                    max_cost_microusd: Some(10_000),
                    max_attempts: Some(3),
                }),
            })
            .expect("start run");
        store
            .apply(AgentRunAction::StartAttempt {
                run_id: run_id.clone(),
                attempt_id: attempt_id.clone(),
                identity: identity("home-a", "gpt-a"),
                transition: AttemptTransition {
                    kind: AttemptTransitionKind::Initial,
                    from_attempt_id: None,
                },
                route_reason: Some("primary coding Home".to_owned()),
                route_decision_id: None,
            })
            .expect("start attempt");
        (task_id, run_id, attempt_id)
    }

    #[test]
    fn lifecycle_projects_cost_failure_and_final_artifact() {
        let temp = TempDir::new().expect("temp");
        let store = AgentRunStore::new(temp.path().join("events.jsonl"));
        let (_, run_id, attempt_id) = setup(&store);
        store
            .apply(AgentRunAction::LinkThread {
                run_id: run_id.clone(),
                attempt_id: Some(attempt_id.clone()),
                thread_id: "thread-1".to_owned(),
                parent_thread_id: None,
                fork_id: None,
            })
            .expect("link");
        store
            .apply(AgentRunAction::RecordArtifact {
                run_id: run_id.clone(),
                attempt_id: attempt_id.clone(),
                thread_id: Some("thread-1".to_owned()),
                artifact_id: "artifact-1".to_owned(),
                artifact_kind: "patch".to_owned(),
            })
            .expect("artifact");
        store
            .apply(AgentRunAction::RecordVerification {
                run_id: run_id.clone(),
                attempt_id: attempt_id.clone(),
                thread_id: Some("thread-1".to_owned()),
                verification_id: "verification-1".to_owned(),
                verification_kind: "tests".to_owned(),
                target_artifact_id: Some("artifact-1".to_owned()),
                succeeded: true,
                duration_ms: 300,
            })
            .expect("verification");
        store
            .apply(AgentRunAction::CompleteAttempt {
                run_id: run_id.clone(),
                attempt_id,
                thread_id: Some("thread-1".to_owned()),
                usage: UsageDelta {
                    input_tokens: 400,
                    output_tokens: 100,
                    cached_input_tokens: 300,
                    duration_ms: 2_000,
                    estimated_cost_microusd: 700,
                    ..UsageDelta::default()
                },
            })
            .expect("complete attempt");
        store
            .apply(AgentRunAction::CompleteRun {
                run_id: run_id.clone(),
                final_artifact_ids: vec!["artifact-1".to_owned()],
            })
            .expect("complete run");

        let report = store.report().expect("report");
        let run = report.run(&run_id).expect("run");
        assert_eq!(run.status, RunLifecycleStatus::Succeeded);
        assert_eq!(run.consumption.total_tokens, 500);
        assert_eq!(run.final_artifact_ids, ["artifact-1"]);
        assert_eq!(run.verifications.len(), 1);
        assert_eq!(report.tasks[0].status, TaskLifecycleStatus::Succeeded);
    }

    #[test]
    fn retry_and_migration_preserve_run_and_enforce_budget() {
        let temp = TempDir::new().expect("temp");
        let store = AgentRunStore::new(temp.path().join("events.jsonl"));
        let (_, run_id, attempt_id) = setup(&store);
        store
            .apply(AgentRunAction::FailAttempt {
                run_id: run_id.clone(),
                attempt_id: attempt_id.clone(),
                thread_id: None,
                status: ObservabilityStatus::Failed,
                usage: UsageDelta {
                    input_tokens: 200,
                    output_tokens: 50,
                    duration_ms: 500,
                    estimated_cost_microusd: 100,
                    ..UsageDelta::default()
                },
                failure: FailureInfo {
                    code: "rate_limited".to_owned(),
                    phase: Some("inference".to_owned()),
                    reason: "provider returned a retryable limit".to_owned(),
                    retryable: true,
                },
            })
            .expect("fail");
        store
            .apply(AgentRunAction::StartAttempt {
                run_id: run_id.clone(),
                attempt_id: "attempt-2".to_owned(),
                identity: identity("home-b", "gpt-b"),
                transition: AttemptTransition {
                    kind: AttemptTransitionKind::Migration,
                    from_attempt_id: Some(attempt_id),
                },
                route_reason: Some("move away from limited account".to_owned()),
                route_decision_id: None,
            })
            .expect("migrate");
        store
            .apply(AgentRunAction::FailAttempt {
                run_id: run_id.clone(),
                attempt_id: "attempt-2".to_owned(),
                thread_id: None,
                status: ObservabilityStatus::Failed,
                usage: UsageDelta {
                    input_tokens: 600,
                    output_tokens: 200,
                    duration_ms: 1_000,
                    estimated_cost_microusd: 200,
                    ..UsageDelta::default()
                },
                failure: FailureInfo {
                    code: "verification_failed".to_owned(),
                    phase: Some("test".to_owned()),
                    reason: "tests did not pass".to_owned(),
                    retryable: true,
                },
            })
            .expect("fail migration");

        let report = store.report().expect("report");
        let run = report.run(&run_id).expect("run");
        assert_eq!(run.consumption.failed_attempt_tokens, 1_050);
        assert!(!run.budget_state.within_budget);
        assert_eq!(run.recovery.recommended_action, "pause");
        let error = store
            .apply(AgentRunAction::StartAttempt {
                run_id,
                attempt_id: "attempt-3".to_owned(),
                identity: identity("home-c", "gpt-c"),
                transition: AttemptTransition {
                    kind: AttemptTransitionKind::Retry,
                    from_attempt_id: Some("attempt-2".to_owned()),
                },
                route_reason: Some("retry after fixing test environment".to_owned()),
                route_decision_id: None,
            })
            .expect_err("budget blocks retry");
        assert!(error.to_string().contains("budget is exhausted"));
    }

    #[test]
    fn route_decision_is_projected_and_attempt_identity_must_match() {
        let temp = TempDir::new().expect("temp");
        let store = AgentRunStore::new(temp.path().join("events.jsonl"));
        store
            .apply(AgentRunAction::CreateTask {
                task_id: "task-route".to_owned(),
                label: "Route a small fix".to_owned(),
                kind: Some("simple_code".to_owned()),
            })
            .expect("task");
        store
            .apply(AgentRunAction::StartRun {
                task_id: "task-route".to_owned(),
                run_id: "run-route".to_owned(),
                budget: None,
            })
            .expect("run");
        let route = store
            .apply(AgentRunAction::RecordRouteDecision {
                run_id: "run-route".to_owned(),
                decision: Box::new(route_decision("home-spark", "spark-model")),
            })
            .expect("route");

        let mismatch = store
            .apply(AgentRunAction::StartAttempt {
                run_id: "run-route".to_owned(),
                attempt_id: "attempt-wrong".to_owned(),
                identity: identity("home-primary", "strong-model"),
                transition: AttemptTransition {
                    kind: AttemptTransitionKind::Initial,
                    from_attempt_id: None,
                },
                route_reason: Some("ignore the decision".to_owned()),
                route_decision_id: Some(route.event_id.clone()),
            })
            .expect_err("mismatched route identity");
        assert!(mismatch
            .to_string()
            .contains("does not match routeDecisionId"));

        store
            .apply(AgentRunAction::StartAttempt {
                run_id: "run-route".to_owned(),
                attempt_id: "attempt-spark".to_owned(),
                identity: identity("home-spark", "spark-model"),
                transition: AttemptTransition {
                    kind: AttemptTransitionKind::Initial,
                    from_attempt_id: None,
                },
                route_reason: Some("follow recorded decision".to_owned()),
                route_decision_id: Some(route.event_id.clone()),
            })
            .expect("matching attempt");
        let report = store.report().expect("report");
        let run = report.run("run-route").expect("run");
        assert_eq!(run.route_decisions.len(), 1);
        assert_eq!(run.route_decisions[0].event_id, route.event_id);
        assert_eq!(
            run.attempts[0].route_decision_id.as_deref(),
            Some(run.route_decisions[0].event_id.as_str())
        );
    }

    #[test]
    fn decide_route_evaluates_and_records_under_one_store_snapshot() {
        let temp = TempDir::new().expect("temp");
        let store = AgentRunStore::new(temp.path().join("events.jsonl"));
        store
            .apply(AgentRunAction::CreateTask {
                task_id: "task-atomic-route".to_owned(),
                label: "Route atomically".to_owned(),
                kind: Some("simple_code".to_owned()),
            })
            .expect("task");
        store
            .apply(AgentRunAction::StartRun {
                task_id: "task-atomic-route".to_owned(),
                run_id: "run-atomic-route".to_owned(),
                budget: None,
            })
            .expect("run");
        let request = RouteRequest {
            schema_version: crate::ROUTE_REQUEST_SCHEMA_VERSION.to_owned(),
            request_id: "request-atomic".to_owned(),
            task_kind: crate::RouteTaskKind::SimpleCode,
            estimated_context_tokens: 2_000,
            estimated_output_tokens: 500,
            required_capabilities: vec!["coding".to_owned()],
            preferred_specialties: vec!["simple_code".to_owned()],
            sensitive_directory: false,
            required_security_domain: None,
            locked_home: None,
            locked_model: None,
            max_estimated_cost_microusd: None,
            allow_degraded: false,
        };
        let homes = vec![RouteHomeState {
            home_id: "home-spark".to_owned(),
            home_alias: Some("spark".to_owned()),
            available: true,
        }];

        let (decision, mutation) = store
            .decide_route("run-atomic-route", &route_policy(), &request, &homes)
            .expect("atomic route decision");

        assert!(decision.ok);
        assert_eq!(decision.observed_event_count, 2);
        assert_eq!(
            mutation.event_id,
            mutation.run.as_ref().unwrap().route_decisions[0].event_id
        );
        let recorded = &mutation.run.as_ref().unwrap().route_decisions[0];
        assert_eq!(recorded.observed_event_count, 2);
        assert_eq!(
            recorded.evaluated_at_timestamp_ms,
            decision.evaluated_at_timestamp_ms
        );
        assert_eq!(recorded.decided_at_ms, decision.evaluated_at_timestamp_ms);
        assert_eq!(recorded.request, request);
        assert_eq!(recorded.candidates[0].active_attempts, 0);
        assert!(!recorded.candidates[0].components[0].reason.is_empty());
        let summary = ObservabilityStore::new(store.path().to_path_buf())
            .summary(&crate::ObservabilityFilter::default())
            .expect("observability summary");
        assert_eq!(summary.totals.route_decisions, 1);
    }
}
