use crate::{
    ExecutionIdentity, SafetySummary, WorktreeConflictDetails, WorktreeConflictDisposition,
    WorktreeEvidenceDetails,
};
use anyhow::{bail, Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::NamedTempFile;

pub const WORKTREE_PLAN_SCHEMA_VERSION: &str = "codexhome.worktree-plan.v1";
pub const WORKTREE_PREPARE_SCHEMA_VERSION: &str = "codexhome.worktree-prepare.v1";
pub const WORKTREE_EVIDENCE_SCHEMA_VERSION: &str = "codexhome.worktree-evidence.v1";
pub const WORKTREE_CONFLICT_SCHEMA_VERSION: &str = "codexhome.worktree-conflict.v1";
const MAX_TEST_LOG_BYTES: usize = 16 * 1024 * 1024;
pub const DEFAULT_TEST_TIMEOUT_MS: u64 = 15 * 60 * 1_000;

#[derive(Debug, Clone)]
pub struct WorktreeAssignment {
    pub task_id: String,
    pub task_label: String,
    pub run_id: String,
    pub attempt_id: String,
    pub identity: ExecutionIdentity,
}

#[derive(Debug, Clone)]
pub struct WorktreePrepareOptions {
    pub repository: PathBuf,
    pub worktree_path: Option<PathBuf>,
    pub base_ref: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreePlan {
    pub ok: bool,
    pub schema_version: &'static str,
    pub task_id: String,
    pub run_id: String,
    pub attempt_id: String,
    pub repository_root: String,
    pub worktree_path: String,
    pub branch: String,
    pub base_ref: String,
    pub base_commit: String,
    pub responsible_identity: ExecutionIdentity,
    pub safety: SafetySummary,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreePrepareReport {
    pub ok: bool,
    pub schema_version: &'static str,
    pub created: bool,
    pub head_commit: String,
    pub plan: WorktreePlan,
    pub safety: SafetySummary,
}

#[derive(Debug, Clone)]
pub struct WorktreeEvidenceOptions {
    pub evidence_id: String,
    pub evidence_root: PathBuf,
    pub test_label: String,
    pub test_program: String,
    pub test_args: Vec<String>,
    pub test_timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeEvidenceReport {
    pub ok: bool,
    pub schema_version: &'static str,
    pub details: WorktreeEvidenceDetails,
    pub safety: SafetySummary,
}

#[derive(Debug, Clone)]
pub struct WorktreeConflictOptions {
    pub check_id: String,
    pub evidence_id: String,
    pub target_ref: String,
    pub on_conflict: WorktreeConflictDisposition,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeConflictReport {
    pub ok: bool,
    pub schema_version: &'static str,
    pub details: WorktreeConflictDetails,
    pub safety: SafetySummary,
}

#[derive(Debug, Default)]
pub struct GitWorktreeManager;

impl GitWorktreeManager {
    pub fn plan(
        assignment: &WorktreeAssignment,
        options: &WorktreePrepareOptions,
    ) -> Result<WorktreePlan> {
        validate_assignment(assignment)?;
        validate_git_ref("baseRef", &options.base_ref)?;
        let repository_root = repository_root(&options.repository)?;
        let base_commit = git_stdout(&repository_root, ["rev-parse", &options.base_ref])?;
        validate_commit_id(&base_commit)?;
        let branch = generated_branch(&assignment.task_label, &assignment.run_id);
        ensure_branch_absent(&repository_root, &branch)?;
        let worktree_path = match &options.worktree_path {
            Some(path) => absolute_new_path(path)?,
            None => default_worktree_path(&repository_root, &assignment.run_id)?,
        };
        if worktree_path.starts_with(&repository_root) {
            bail!("worktree path must be outside the source repository");
        }
        ensure_new_worktree_path(&worktree_path)?;

        Ok(WorktreePlan {
            ok: true,
            schema_version: WORKTREE_PLAN_SCHEMA_VERSION,
            task_id: assignment.task_id.clone(),
            run_id: assignment.run_id.clone(),
            attempt_id: assignment.attempt_id.clone(),
            repository_root: repository_root.to_string_lossy().into_owned(),
            worktree_path: worktree_path.to_string_lossy().into_owned(),
            branch,
            base_ref: options.base_ref.clone(),
            base_commit,
            responsible_identity: assignment.identity.clone(),
            safety: worktree_safety(),
        })
    }

    pub fn prepare(plan: WorktreePlan) -> Result<WorktreePrepareReport> {
        let repository_root = PathBuf::from(&plan.repository_root);
        let worktree_path = PathBuf::from(&plan.worktree_path);
        ensure_branch_absent(&repository_root, &plan.branch)?;
        ensure_new_worktree_path(&worktree_path)?;
        if let Some(parent) = worktree_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }

        let output = git_output(
            &repository_root,
            [
                "worktree",
                "add",
                "-b",
                plan.branch.as_str(),
                plan.worktree_path.as_str(),
                plan.base_commit.as_str(),
            ],
        )?;
        if !output.status.success() {
            let failure = bounded_stderr(&output.stderr);
            return match Self::rollback(&plan) {
                Ok(()) => Err(anyhow::anyhow!("git worktree add failed: {failure}")),
                Err(rollback) => Err(anyhow::anyhow!(
                    "git worktree add failed: {failure}; rollback also failed: {rollback:#}"
                )),
            };
        }

        let verification = (|| -> Result<String> {
            let head = git_stdout(&worktree_path, ["rev-parse", "HEAD"])?;
            if head != plan.base_commit {
                bail!(
                    "prepared worktree HEAD '{}' differs from planned base '{}'",
                    head,
                    plan.base_commit
                );
            }
            let branch = git_stdout(&worktree_path, ["branch", "--show-current"])?;
            if branch != plan.branch {
                bail!(
                    "prepared worktree branch '{}' differs from planned branch '{}'",
                    branch,
                    plan.branch
                );
            }
            Ok(head)
        })();

        let head_commit =
            match verification {
                Ok(head) => head,
                Err(error) => {
                    let rollback = Self::rollback(&plan);
                    return match rollback {
                        Ok(()) => Err(error),
                        Err(rollback_error) => Err(error
                            .context(format!("worktree rollback also failed: {rollback_error:#}"))),
                    };
                }
            };

        Ok(WorktreePrepareReport {
            ok: true,
            schema_version: WORKTREE_PREPARE_SCHEMA_VERSION,
            created: true,
            head_commit,
            plan,
            safety: worktree_safety(),
        })
    }

    pub fn rollback(plan: &WorktreePlan) -> Result<()> {
        let repository_root = PathBuf::from(&plan.repository_root);
        let worktree_path = PathBuf::from(&plan.worktree_path);
        if worktree_path.exists() {
            let remove = git_output(
                &repository_root,
                ["worktree", "remove", "--force", plan.worktree_path.as_str()],
            )?;
            if !remove.status.success() {
                bail!(
                    "failed to remove newly created worktree: {}",
                    bounded_stderr(&remove.stderr)
                );
            }
        }
        if branch_exists(&repository_root, &plan.branch)? {
            let delete = git_output(&repository_root, ["branch", "-D", plan.branch.as_str()])?;
            if !delete.status.success() {
                bail!(
                    "failed to delete newly created branch: {}",
                    bounded_stderr(&delete.stderr)
                );
            }
        }
        Ok(())
    }

    pub fn collect_evidence(
        plan: &WorktreePlan,
        options: &WorktreeEvidenceOptions,
    ) -> Result<WorktreeEvidenceReport> {
        validate_identifier("evidenceId", &options.evidence_id)?;
        if options.test_label.trim().is_empty() || options.test_label.len() > 256 {
            bail!("testLabel must contain 1..=256 bytes");
        }
        if options.test_program.trim().is_empty()
            || options.test_program.contains(['\n', '\r', '\0'])
        {
            bail!("testProgram is invalid");
        }
        if options.test_timeout_ms == 0 || options.test_timeout_ms > 24 * 60 * 60 * 1_000 {
            bail!("testTimeoutMs must be within 1..=86400000");
        }
        let worktree_path = verify_prepared_worktree(plan)?;
        let head_commit = git_stdout(&worktree_path, ["rev-parse", "HEAD"])?;
        validate_commit_id(&head_commit)?;
        let commit_count = git_stdout(
            &worktree_path,
            [
                "rev-list",
                "--count",
                &format!("{}..HEAD", plan.base_commit),
            ],
        )?
        .parse::<u32>()
        .context("Git commit count is not a u32")?;
        if commit_count == 0 || head_commit == plan.base_commit {
            bail!("worktree evidence requires at least one committed change");
        }

        let patch = git_bytes(
            &worktree_path,
            [
                "diff",
                "--binary",
                &format!("{}..{}", plan.base_commit, head_commit),
            ],
        )?;
        if patch.is_empty() {
            bail!("worktree evidence patch is empty");
        }
        let (changed_files, insertions, deletions) =
            diff_statistics(&worktree_path, &plan.base_commit, &head_commit)?;

        let started = Instant::now();
        let stdout_file = NamedTempFile::new().context("failed to create temporary test stdout")?;
        let stderr_file = NamedTempFile::new().context("failed to create temporary test stderr")?;
        let mut child = Command::new(&options.test_program)
            .args(&options.test_args)
            .current_dir(&worktree_path)
            .stdin(Stdio::null())
            .stdout(Stdio::from(
                stdout_file
                    .as_file()
                    .try_clone()
                    .context("failed to clone temporary test stdout")?,
            ))
            .stderr(Stdio::from(
                stderr_file
                    .as_file()
                    .try_clone()
                    .context("failed to clone temporary test stderr")?,
            ))
            .spawn()
            .with_context(|| format!("failed to execute test '{}'", options.test_label))?;
        let deadline = started + Duration::from_millis(options.test_timeout_ms);
        let (test_status, timed_out) = loop {
            if let Some(status) = child
                .try_wait()
                .context("failed to poll worktree evidence test")?
            {
                break (status, false);
            }
            if Instant::now() >= deadline {
                child.kill().context("failed to terminate timed-out test")?;
                let status = child
                    .wait()
                    .context("failed to reap timed-out evidence test")?;
                break (status, true);
            }
            thread::sleep(Duration::from_millis(25));
        };
        let test_duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let test_exit_code = if timed_out {
            -1
        } else {
            test_status.code().unwrap_or(-1)
        };
        let stdout =
            fs::read(stdout_file.path()).context("failed to read temporary test stdout")?;
        let stderr =
            fs::read(stderr_file.path()).context("failed to read temporary test stderr")?;
        let mut test_log = Vec::with_capacity(stdout.len().saturating_add(stderr.len()));
        test_log.extend_from_slice(&stdout);
        if !stdout.is_empty() && !stderr.is_empty() {
            test_log.push(b'\n');
        }
        test_log.extend_from_slice(&stderr);
        if timed_out {
            test_log.extend_from_slice(
                format!(
                    "\n[codexhome] test timed out after {} ms\n",
                    options.test_timeout_ms
                )
                .as_bytes(),
            );
        }
        test_log.truncate(MAX_TEST_LOG_BYTES);

        let status = git_bytes(&worktree_path, ["status", "--porcelain=v1"])?;
        let worktree_clean = status.is_empty();
        if !options.evidence_root.is_absolute() {
            bail!("evidenceRoot must be absolute");
        }
        fs::create_dir_all(&options.evidence_root).with_context(|| {
            format!(
                "failed to create evidence root {}",
                options.evidence_root.display()
            )
        })?;
        let evidence_root = options
            .evidence_root
            .canonicalize()
            .context("failed to canonicalize evidence root")?;
        let repository_root = PathBuf::from(&plan.repository_root);
        if evidence_root.starts_with(&worktree_path) || evidence_root.starts_with(&repository_root)
        {
            bail!("evidenceRoot must be outside the source repository and worktree");
        }
        let evidence_dir = evidence_root.join(&plan.run_id).join(&options.evidence_id);
        fs::create_dir_all(&evidence_dir)
            .with_context(|| format!("failed to create {}", evidence_dir.display()))?;
        let patch_path = evidence_dir.join("changes.patch");
        let test_log_path = evidence_dir.join("test.log");
        write_private_file(&patch_path, &patch)?;
        write_private_file(&test_log_path, &test_log)?;

        Ok(WorktreeEvidenceReport {
            ok: true,
            schema_version: WORKTREE_EVIDENCE_SCHEMA_VERSION,
            details: WorktreeEvidenceDetails {
                evidence_id: options.evidence_id.clone(),
                base_commit: plan.base_commit.clone(),
                head_commit,
                commit_count,
                diff_sha256: sha256_hex(&patch),
                patch_path: patch_path.to_string_lossy().into_owned(),
                patch_bytes: u64::try_from(patch.len()).unwrap_or(u64::MAX),
                changed_files,
                insertions,
                deletions,
                worktree_clean,
                tests_succeeded: !timed_out && test_status.success(),
                test_label: options.test_label.clone(),
                test_exit_code,
                test_duration_ms,
                test_output_sha256: sha256_hex(&test_log),
                test_log_path: test_log_path.to_string_lossy().into_owned(),
            },
            safety: worktree_safety(),
        })
    }

    pub fn check_conflicts(
        plan: &WorktreePlan,
        options: &WorktreeConflictOptions,
    ) -> Result<WorktreeConflictReport> {
        validate_identifier("checkId", &options.check_id)?;
        validate_identifier("evidenceId", &options.evidence_id)?;
        validate_git_ref("targetRef", &options.target_ref)?;
        if options.on_conflict == WorktreeConflictDisposition::None {
            bail!("onConflict must be human_required or replan_requested");
        }
        let worktree_path = verify_prepared_worktree(plan)?;
        let head_commit = git_stdout(&worktree_path, ["rev-parse", "HEAD"])?;
        let target_commit = git_stdout(&worktree_path, ["rev-parse", &options.target_ref])?;
        validate_commit_id(&head_commit)?;
        validate_commit_id(&target_commit)?;
        let output = git_output(
            &worktree_path,
            [
                "merge-tree",
                "--write-tree",
                "--name-only",
                target_commit.as_str(),
                head_commit.as_str(),
            ],
        )?;
        let conflict_free = match output.status.code() {
            Some(0) => true,
            Some(1) => false,
            _ => bail!("git merge-tree failed: {}", bounded_stderr(&output.stderr)),
        };
        let mut transcript = output.stdout;
        transcript.extend_from_slice(&output.stderr);
        let conflicting_paths_count = if conflict_free {
            0
        } else {
            count_conflict_paths(&transcript)
        };
        Ok(WorktreeConflictReport {
            ok: true,
            schema_version: WORKTREE_CONFLICT_SCHEMA_VERSION,
            details: WorktreeConflictDetails {
                check_id: options.check_id.clone(),
                evidence_id: options.evidence_id.clone(),
                target_ref: options.target_ref.clone(),
                target_commit,
                head_commit,
                conflict_free,
                disposition: if conflict_free {
                    WorktreeConflictDisposition::None
                } else {
                    options.on_conflict
                },
                conflicting_paths_count,
                conflicts_sha256: sha256_hex(&transcript),
            },
            safety: worktree_safety(),
        })
    }

    pub fn verify_current(
        details: &crate::WorktreePreparedDetails,
        expected_head: &str,
    ) -> Result<()> {
        validate_commit_id(expected_head)?;
        let worktree_path = PathBuf::from(&details.worktree_path)
            .canonicalize()
            .with_context(|| format!("prepared worktree is missing: {}", details.worktree_path))?;
        let branch = git_stdout(&worktree_path, ["branch", "--show-current"])?;
        if branch != details.branch {
            bail!(
                "worktree branch '{}' differs from recorded branch '{}'",
                branch,
                details.branch
            );
        }
        let head = git_stdout(&worktree_path, ["rev-parse", "HEAD"])?;
        if head != expected_head {
            bail!(
                "worktree HEAD '{}' differs from reviewed evidence HEAD '{}'",
                head,
                expected_head
            );
        }
        if !git_bytes(&worktree_path, ["status", "--porcelain=v1"])?.is_empty() {
            bail!("worktree has uncommitted changes after its latest evidence");
        }
        Ok(())
    }
}

fn validate_assignment(assignment: &WorktreeAssignment) -> Result<()> {
    for (name, value) in [
        ("taskId", assignment.task_id.as_str()),
        ("runId", assignment.run_id.as_str()),
        ("attemptId", assignment.attempt_id.as_str()),
    ] {
        validate_identifier(name, value)?;
    }
    if assignment.task_label.trim().is_empty() || assignment.task_label.len() > 256 {
        bail!("taskLabel must contain 1..=256 bytes");
    }
    if assignment.identity.home_id.is_none() || assignment.identity.model.is_none() {
        bail!("worktree assignment requires responsible Home and model");
    }
    Ok(())
}

fn verify_prepared_worktree(plan: &WorktreePlan) -> Result<PathBuf> {
    let repository_root = PathBuf::from(&plan.repository_root);
    let worktree_path = PathBuf::from(&plan.worktree_path)
        .canonicalize()
        .with_context(|| format!("prepared worktree is missing: {}", plan.worktree_path))?;
    let registered = git_stdout(
        &repository_root,
        [
            "-C",
            plan.worktree_path.as_str(),
            "rev-parse",
            "--show-toplevel",
        ],
    )?;
    let registered = PathBuf::from(registered)
        .canonicalize()
        .context("failed to canonicalize registered worktree")?;
    if registered != worktree_path {
        bail!("worktree path is not registered with the planned repository");
    }
    let branch = git_stdout(&worktree_path, ["branch", "--show-current"])?;
    if branch != plan.branch {
        bail!(
            "worktree branch '{}' differs from planned branch '{}'",
            branch,
            plan.branch
        );
    }
    Ok(worktree_path)
}

fn repository_root(repository: &Path) -> Result<PathBuf> {
    let path = repository
        .canonicalize()
        .with_context(|| format!("repository does not exist: {}", repository.display()))?;
    let root = git_stdout(&path, ["rev-parse", "--show-toplevel"])?;
    let root = PathBuf::from(root)
        .canonicalize()
        .context("failed to canonicalize Git repository root")?;
    if path != root {
        bail!(
            "repository must be the Git top-level directory '{}'",
            root.display()
        );
    }
    Ok(root)
}

fn generated_branch(task_label: &str, run_id: &str) -> String {
    let mut slug = task_label
        .chars()
        .flat_map(char::to_lowercase)
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    while slug.contains("--") {
        slug = slug.replace("--", "-");
    }
    slug = slug.trim_matches('-').to_owned();
    if slug.is_empty() {
        slug = "task".to_owned();
    }
    slug.truncate(48);
    let digest = Sha256::digest(run_id.as_bytes());
    let suffix = format!("{digest:x}");
    format!("codex/{}-{}", slug.trim_matches('-'), &suffix[..10])
}

fn default_worktree_path(repository_root: &Path, run_id: &str) -> Result<PathBuf> {
    let parent = repository_root
        .parent()
        .context("repository root has no parent directory")?;
    let repository_name = repository_root
        .file_name()
        .and_then(|value| value.to_str())
        .context("repository root has no UTF-8 directory name")?;
    let digest = Sha256::digest(run_id.as_bytes());
    let suffix = format!("{digest:x}");
    Ok(parent
        .join(".codexhome-worktrees")
        .join(repository_name)
        .join(format!("run-{}", &suffix[..12])))
}

fn absolute_new_path(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        bail!("worktree path must be absolute");
    }
    let parent = path
        .parent()
        .context("worktree path has no parent directory")?;
    let canonical_parent = parent
        .canonicalize()
        .with_context(|| format!("worktree parent does not exist: {}", parent.display()))?;
    let name = path
        .file_name()
        .context("worktree path has no final component")?;
    Ok(canonical_parent.join(name))
}

fn ensure_new_worktree_path(path: &Path) -> Result<()> {
    if path.exists() {
        bail!("worktree path already exists: {}", path.display());
    }
    Ok(())
}

fn ensure_branch_absent(repository: &Path, branch: &str) -> Result<()> {
    validate_git_ref("branch", branch)?;
    if branch_exists(repository, branch)? {
        bail!("generated worktree branch already exists: {branch}");
    }
    Ok(())
}

fn branch_exists(repository: &Path, branch: &str) -> Result<bool> {
    let reference = format!("refs/heads/{branch}");
    let output = git_output(
        repository,
        ["show-ref", "--verify", "--quiet", reference.as_str()],
    )?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => bail!(
            "failed to inspect branch '{}': {}",
            branch,
            bounded_stderr(&output.stderr)
        ),
    }
}

fn validate_git_ref(name: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() || value.len() > 256 || value.starts_with('-') {
        bail!("{name} is not a safe Git ref");
    }
    if value.contains(['\n', '\r', '\0']) {
        bail!("{name} contains an unsafe character");
    }
    Ok(())
}

fn validate_identifier(name: &str, value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        bail!("{name} is not a valid identifier");
    }
    Ok(())
}

fn validate_commit_id(value: &str) -> Result<()> {
    if value.len() != 40 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("Git commit ID is not a full 40-character hexadecimal hash");
    }
    Ok(())
}

fn git_stdout<const N: usize>(repository: &Path, args: [&str; N]) -> Result<String> {
    let output = git_output(repository, args)?;
    if !output.status.success() {
        bail!("git command failed: {}", bounded_stderr(&output.stderr));
    }
    String::from_utf8(output.stdout)
        .context("git output is not UTF-8")
        .map(|value| value.trim().to_owned())
}

fn git_bytes<const N: usize>(repository: &Path, args: [&str; N]) -> Result<Vec<u8>> {
    let output = git_output(repository, args)?;
    if !output.status.success() {
        bail!("git command failed: {}", bounded_stderr(&output.stderr));
    }
    Ok(output.stdout)
}

fn git_output<I, S>(repository: &Path, args: I) -> Result<Output>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(args)
        .output()
        .context("failed to execute git")
}

fn bounded_stderr(stderr: &[u8]) -> String {
    let value = String::from_utf8_lossy(stderr);
    value.trim().chars().take(2_048).collect()
}

fn diff_statistics(repository: &Path, base: &str, head: &str) -> Result<(u32, u64, u64)> {
    let range = format!("{base}..{head}");
    let output = git_stdout(repository, ["diff", "--numstat", range.as_str()])?;
    let mut changed_files = 0_u32;
    let mut insertions = 0_u64;
    let mut deletions = 0_u64;
    for line in output.lines().filter(|line| !line.trim().is_empty()) {
        let mut fields = line.splitn(3, '\t');
        let added = fields.next().context("git numstat is missing insertions")?;
        let removed = fields.next().context("git numstat is missing deletions")?;
        fields.next().context("git numstat is missing a path")?;
        changed_files = changed_files
            .checked_add(1)
            .context("changed file count overflow")?;
        if added != "-" {
            insertions = insertions
                .checked_add(added.parse::<u64>().context("invalid insertion count")?)
                .context("insertion count overflow")?;
        }
        if removed != "-" {
            deletions = deletions
                .checked_add(removed.parse::<u64>().context("invalid deletion count")?)
                .context("deletion count overflow")?;
        }
    }
    Ok((changed_files, insertions, deletions))
}

fn write_private_file(path: &Path, contents: &[u8]) -> Result<()> {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .with_context(|| format!("failed to secure {}", path.display()))?;
    }
    file.write_all(contents)
        .with_context(|| format!("failed to write {}", path.display()))?;
    file.sync_all()
        .with_context(|| format!("failed to sync {}", path.display()))
}

fn sha256_hex(contents: &[u8]) -> String {
    format!("{:x}", Sha256::digest(contents))
}

fn count_conflict_paths(transcript: &[u8]) -> u32 {
    let mut paths = BTreeSet::new();
    let transcript = String::from_utf8_lossy(transcript);
    for line in transcript.lines() {
        let value = line.trim();
        if value.is_empty()
            || value.starts_with("CONFLICT")
            || value.starts_with("Auto-merging")
            || (value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        {
            continue;
        }
        paths.insert(value);
    }
    u32::try_from(paths.len().max(1)).unwrap_or(u32::MAX)
}

fn worktree_safety() -> SafetySummary {
    SafetySummary {
        secrets_redacted: true,
        includes_secrets: false,
        reads_auth_contents: false,
        includes_local_paths: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn three_runs_prepare_isolated_worktrees_and_unique_branches() {
        let temp = TempDir::new().expect("temp");
        let repository = temp.path().join("repository");
        init_repository(&repository);
        let mut plans = Vec::new();

        for index in 1..=3 {
            let assignment = assignment(index);
            let plan = GitWorktreeManager::plan(
                &assignment,
                &WorktreePrepareOptions {
                    repository: repository.clone(),
                    worktree_path: None,
                    base_ref: "HEAD".to_owned(),
                },
            )
            .expect("plan");
            let report = GitWorktreeManager::prepare(plan.clone()).expect("prepare");
            assert_eq!(report.head_commit, plan.base_commit);
            assert!(Path::new(&plan.worktree_path).join("README.md").exists());
            fs::write(
                Path::new(&plan.worktree_path).join(format!("home-{index}.txt")),
                format!("isolated change from Home {index}\n"),
            )
            .expect("write isolated file");
            plans.push(plan);
        }

        assert_eq!(
            plans
                .iter()
                .map(|plan| plan.branch.as_str())
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            3
        );
        assert_eq!(
            plans
                .iter()
                .map(|plan| plan.worktree_path.as_str())
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            3
        );
        for (index, plan) in plans.iter().enumerate() {
            for other in 1..=3 {
                let exists = Path::new(&plan.worktree_path)
                    .join(format!("home-{other}.txt"))
                    .exists();
                assert_eq!(exists, other == index + 1);
            }
        }
        assert!(!repository.join("home-1.txt").exists());
        assert!(!repository.join("home-2.txt").exists());
        assert!(!repository.join("home-3.txt").exists());

        for plan in plans.iter().rev() {
            GitWorktreeManager::rollback(plan).expect("rollback");
        }
    }

    #[test]
    fn plan_rejects_existing_generated_branch_and_path() {
        let temp = TempDir::new().expect("temp");
        let repository = temp.path().join("repository");
        init_repository(&repository);
        let assignment = assignment(1);
        let plan = GitWorktreeManager::plan(
            &assignment,
            &WorktreePrepareOptions {
                repository: repository.clone(),
                worktree_path: None,
                base_ref: "HEAD".to_owned(),
            },
        )
        .expect("plan");
        GitWorktreeManager::prepare(plan.clone()).expect("prepare");

        let error = GitWorktreeManager::plan(
            &assignment,
            &WorktreePrepareOptions {
                repository: repository.clone(),
                worktree_path: None,
                base_ref: "HEAD".to_owned(),
            },
        )
        .expect_err("existing branch must fail");
        assert!(error.to_string().contains("branch already exists"));
        GitWorktreeManager::rollback(&plan).expect("rollback");

        let nested = GitWorktreeManager::plan(
            &assignment,
            &WorktreePrepareOptions {
                repository: repository.clone(),
                worktree_path: Some(repository.join("nested-worktree")),
                base_ref: "HEAD".to_owned(),
            },
        )
        .expect_err("nested worktree path must fail");
        assert!(nested.to_string().contains("outside the source repository"));
    }

    #[test]
    fn committed_changes_produce_tested_evidence_and_clean_conflict_check() {
        let temp = TempDir::new().expect("temp");
        let repository = temp.path().join("repository");
        init_repository(&repository);
        let plan = GitWorktreeManager::plan(
            &assignment(1),
            &WorktreePrepareOptions {
                repository,
                worktree_path: None,
                base_ref: "HEAD".to_owned(),
            },
        )
        .expect("plan");
        GitWorktreeManager::prepare(plan.clone()).expect("prepare");
        let worktree = Path::new(&plan.worktree_path);
        fs::write(worktree.join("feature.txt"), "implemented\n").expect("feature");
        run_git(worktree, ["add", "feature.txt"]);
        run_git(worktree, ["commit", "-m", "implement feature"]);

        let evidence = GitWorktreeManager::collect_evidence(
            &plan,
            &WorktreeEvidenceOptions {
                evidence_id: "evidence-1".to_owned(),
                evidence_root: temp.path().join("evidence"),
                test_label: "true smoke".to_owned(),
                test_program: "/usr/bin/true".to_owned(),
                test_args: Vec::new(),
                test_timeout_ms: DEFAULT_TEST_TIMEOUT_MS,
            },
        )
        .expect("evidence");
        assert_eq!(evidence.details.commit_count, 1);
        assert_eq!(evidence.details.changed_files, 1);
        assert!(evidence.details.worktree_clean);
        assert!(evidence.details.tests_succeeded);
        assert!(Path::new(&evidence.details.patch_path).exists());
        assert!(Path::new(&evidence.details.test_log_path).exists());

        let conflict = GitWorktreeManager::check_conflicts(
            &plan,
            &WorktreeConflictOptions {
                check_id: "check-1".to_owned(),
                evidence_id: evidence.details.evidence_id,
                target_ref: plan.base_commit.clone(),
                on_conflict: WorktreeConflictDisposition::HumanRequired,
            },
        )
        .expect("conflict check");
        assert!(conflict.details.conflict_free);
        assert_eq!(
            conflict.details.disposition,
            WorktreeConflictDisposition::None
        );
        GitWorktreeManager::rollback(&plan).expect("rollback");
    }

    #[test]
    fn divergent_same_file_change_requires_explicit_conflict_disposition() {
        let temp = TempDir::new().expect("temp");
        let repository = temp.path().join("repository");
        init_repository(&repository);
        let plan = GitWorktreeManager::plan(
            &assignment(1),
            &WorktreePrepareOptions {
                repository: repository.clone(),
                worktree_path: None,
                base_ref: "HEAD".to_owned(),
            },
        )
        .expect("plan");
        GitWorktreeManager::prepare(plan.clone()).expect("prepare");
        let worktree = Path::new(&plan.worktree_path);
        fs::write(worktree.join("README.md"), "worktree version\n").expect("worktree change");
        run_git(worktree, ["add", "README.md"]);
        run_git(worktree, ["commit", "-m", "change from worktree"]);
        fs::write(repository.join("README.md"), "main version\n").expect("main change");
        run_git(&repository, ["add", "README.md"]);
        run_git(&repository, ["commit", "-m", "change from main"]);
        let target_commit = git_stdout(&repository, ["rev-parse", "HEAD"]).expect("target commit");

        let conflict = GitWorktreeManager::check_conflicts(
            &plan,
            &WorktreeConflictOptions {
                check_id: "check-conflict".to_owned(),
                evidence_id: "evidence-conflict".to_owned(),
                target_ref: target_commit,
                on_conflict: WorktreeConflictDisposition::ReplanRequested,
            },
        )
        .expect("conflict check");
        assert!(!conflict.details.conflict_free);
        assert_eq!(
            conflict.details.disposition,
            WorktreeConflictDisposition::ReplanRequested
        );
        assert!(conflict.details.conflicting_paths_count >= 1);
        GitWorktreeManager::rollback(&plan).expect("rollback");
    }

    fn assignment(index: usize) -> WorktreeAssignment {
        WorktreeAssignment {
            task_id: format!("task-{index}"),
            task_label: "Parallel implementation".to_owned(),
            run_id: format!("run-{index}"),
            attempt_id: format!("attempt-{index}"),
            identity: ExecutionIdentity {
                home_id: Some(format!("home-{index}")),
                home_alias: Some(format!("@home-{index}")),
                account_id: Some(format!("account-{index}")),
                provider: Some("test".to_owned()),
                model: Some(format!("model-{index}")),
                agent_role: Some("executor".to_owned()),
            },
        }
    }

    fn init_repository(path: &Path) {
        fs::create_dir_all(path).expect("repository directory");
        run_git(path, ["init"]);
        run_git(path, ["config", "user.name", "CodexHome Test"]);
        run_git(path, ["config", "user.email", "test@example.invalid"]);
        fs::write(path.join("README.md"), "initial\n").expect("seed file");
        run_git(path, ["add", "README.md"]);
        run_git(path, ["commit", "-m", "initial"]);
    }

    fn run_git<const N: usize>(path: &Path, args: [&str; N]) {
        let output = git_output(path, args).expect("git");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
