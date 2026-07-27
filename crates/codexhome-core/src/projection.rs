use crate::{
    compute_skill_digest, EffectiveRuntimeManifest, RuntimeIsolation, SafetySummary,
    EFFECTIVE_RUNTIME_SCHEMA_VERSION,
};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Component, Path, PathBuf};

pub const RUNTIME_PROJECTION_SCHEMA_VERSION: &str = "codexhome.runtime-projection.v1";
const COPY_OWNERSHIP_MARKER: &str = ".codexhome-projection.json";

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionMode {
    #[default]
    Symlink,
    Copy,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionEntryKind {
    Skill,
    AccountAuth,
    AccountConfig,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionOperation {
    Create,
    Replace,
    Keep,
    SkipMissingOptional,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectionEntry {
    pub kind: ProjectionEntryKind,
    pub source_path: String,
    pub target_path: String,
    pub mode: ProjectionMode,
    pub operation: ProjectionOperation,
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeProjectionReport {
    pub ok: bool,
    pub schema_version: &'static str,
    pub manifest_id: String,
    pub dry_run: bool,
    pub verified: bool,
    pub entries: Vec<ProjectionEntry>,
    pub safety: SafetySummary,
}

pub struct RuntimeProjection;

impl RuntimeProjection {
    pub fn plan(
        manifest: &EffectiveRuntimeManifest,
        mode: ProjectionMode,
    ) -> Result<RuntimeProjectionReport> {
        validate_manifest_paths(manifest)?;
        let mut entries = Vec::new();
        for skill in &manifest.skills {
            let source = Path::new(&skill.source_path);
            if source.exists() {
                let digest = compute_skill_digest(source)?;
                if !digest.eq_ignore_ascii_case(&skill.digest_sha256) {
                    bail!(
                        "Skill '{}' digest mismatch: expected {}, observed {}",
                        skill.id,
                        skill.digest_sha256,
                        digest
                    );
                }
            }
            entries.push(plan_entry(
                ProjectionEntryKind::Skill,
                source,
                Path::new(&skill.target_path),
                mode,
                skill.required,
            )?);
        }

        if manifest.isolation == RuntimeIsolation::Hard {
            let account_home = Path::new(&manifest.account.home_path);
            let runtime_home = Path::new(&manifest.codex_home);
            entries.push(plan_entry(
                ProjectionEntryKind::AccountConfig,
                &account_home.join("config.toml"),
                &runtime_home.join("config.toml"),
                ProjectionMode::Symlink,
                manifest.account.has_config,
            )?);
            entries.push(plan_entry(
                ProjectionEntryKind::AccountAuth,
                &account_home.join("auth.json"),
                &runtime_home.join("auth.json"),
                ProjectionMode::Symlink,
                manifest.account.has_auth,
            )?);
        }

        Ok(RuntimeProjectionReport {
            ok: entries.iter().all(|entry| {
                entry.operation != ProjectionOperation::SkipMissingOptional || !entry.required
            }),
            schema_version: RUNTIME_PROJECTION_SCHEMA_VERSION,
            manifest_id: manifest.manifest_id.clone(),
            dry_run: true,
            verified: false,
            entries,
            safety: SafetySummary {
                secrets_redacted: true,
                includes_secrets: false,
                reads_auth_contents: false,
                includes_local_paths: true,
            },
        })
    }

    pub fn apply(
        manifest: &EffectiveRuntimeManifest,
        mode: ProjectionMode,
        dry_run: bool,
        allow_replace: bool,
    ) -> Result<RuntimeProjectionReport> {
        let mut report = Self::plan(manifest, mode)?;
        if dry_run {
            return Ok(report);
        }
        if !allow_replace
            && report
                .entries
                .iter()
                .any(|entry| entry.operation == ProjectionOperation::Replace)
        {
            bail!("projection would replace existing targets; inspect the plan and pass force");
        }

        let mut created = Vec::<PathBuf>::new();
        let mut backups = Vec::<(PathBuf, PathBuf)>::new();
        let apply_result = (|| -> Result<()> {
            for (index, entry) in report.entries.iter().enumerate() {
                if matches!(
                    entry.operation,
                    ProjectionOperation::Keep | ProjectionOperation::SkipMissingOptional
                ) {
                    continue;
                }
                let source = Path::new(&entry.source_path);
                let target = Path::new(&entry.target_path);
                if entry.operation == ProjectionOperation::Replace {
                    let backup = projection_backup_path(target, index)?;
                    fs::rename(target, &backup).with_context(|| {
                        format!(
                            "failed to stage existing projection {} for replacement",
                            target.display()
                        )
                    })?;
                    backups.push((target.to_path_buf(), backup));
                }
                let parent = target
                    .parent()
                    .context("projection target has no parent directory")?;
                fs::create_dir_all(parent).with_context(|| {
                    format!("failed to create projection directory {}", parent.display())
                })?;
                match entry.mode {
                    ProjectionMode::Symlink => create_symlink(source, target)?,
                    ProjectionMode::Copy => copy_tree(source, target)?,
                }
                created.push(target.to_path_buf());
            }
            Ok(())
        })();

        if let Err(error) = apply_result {
            for path in created.iter().rev() {
                let _ = remove_projection_target(path);
            }
            for (target, backup) in backups.iter().rev() {
                let _ = fs::rename(backup, target);
            }
            return Err(error);
        }
        for (_, backup) in &backups {
            remove_projection_target(backup)?;
        }

        report.dry_run = false;
        report.verified = verify_entries(&report.entries)?;
        report.ok = report.verified;
        Ok(report)
    }

    pub fn verify(
        manifest: &EffectiveRuntimeManifest,
        mode: ProjectionMode,
    ) -> Result<RuntimeProjectionReport> {
        let mut report = Self::plan(manifest, mode)?;
        report.verified = verify_entries(&report.entries)?;
        report.ok = report.verified;
        Ok(report)
    }

    pub fn clean(
        manifest: &EffectiveRuntimeManifest,
        mode: ProjectionMode,
        dry_run: bool,
    ) -> Result<RuntimeProjectionReport> {
        let mut report = Self::plan(manifest, mode)?;
        if !dry_run {
            for entry in report.entries.iter().rev() {
                let target = Path::new(&entry.target_path);
                if target.exists() || target.is_symlink() {
                    ensure_managed_target(manifest, target)?;
                    if !projection_matches(Path::new(&entry.source_path), target, entry.mode)? {
                        bail!(
                            "refusing to clean unowned projection target {}",
                            target.display()
                        );
                    }
                    remove_projection_target(target)?;
                }
            }
        }
        report.dry_run = dry_run;
        report.verified = !report.entries.iter().any(|entry| {
            let target = Path::new(&entry.target_path);
            target.exists() || target.is_symlink()
        });
        report.ok = dry_run || report.verified;
        Ok(report)
    }
}

fn plan_entry(
    kind: ProjectionEntryKind,
    source: &Path,
    target: &Path,
    mode: ProjectionMode,
    required: bool,
) -> Result<ProjectionEntry> {
    if !source.is_absolute() || !target.is_absolute() {
        bail!("projection source and target paths must be absolute");
    }
    let source_exists = source.exists();
    if required && !source_exists {
        bail!(
            "required projection source does not exist: {}",
            source.display()
        );
    }
    if kind == ProjectionEntryKind::Skill && source_exists && !source.is_dir() {
        bail!(
            "Skill projection source must be a directory: {}",
            source.display()
        );
    }
    let operation = if !source_exists {
        ProjectionOperation::SkipMissingOptional
    } else if projection_matches(source, target, mode)? {
        ProjectionOperation::Keep
    } else if target.exists() || target.is_symlink() {
        ProjectionOperation::Replace
    } else {
        ProjectionOperation::Create
    };
    Ok(ProjectionEntry {
        kind,
        source_path: source.to_string_lossy().into_owned(),
        target_path: target.to_string_lossy().into_owned(),
        mode,
        operation,
        required,
    })
}

fn validate_manifest_paths(manifest: &EffectiveRuntimeManifest) -> Result<()> {
    if !manifest.ok || manifest.schema_version != EFFECTIVE_RUNTIME_SCHEMA_VERSION {
        bail!("unsupported or unsuccessful Effective Runtime Manifest");
    }
    let project_root = Path::new(&manifest.project.project_root);
    let runtime_root = Path::new(&manifest.runtime_root);
    let codex_home = Path::new(&manifest.codex_home);
    let account_home = Path::new(&manifest.account.home_path);
    if [project_root, runtime_root, codex_home, account_home]
        .into_iter()
        .any(|path| !is_normalized_absolute(path))
    {
        bail!("effective runtime paths must be normalized absolute paths");
    }
    if manifest.isolation == RuntimeIsolation::Hard && codex_home != runtime_root {
        bail!("hard-isolated codexHome must equal the manifest runtimeRoot");
    }
    for skill in &manifest.skills {
        if !is_normalized_absolute(Path::new(&skill.source_path))
            || !is_normalized_absolute(Path::new(&skill.target_path))
        {
            bail!("Skill source and target paths must be normalized absolute paths");
        }
        ensure_managed_target(manifest, Path::new(&skill.target_path))?;
    }
    if manifest.isolation == RuntimeIsolation::Hard {
        ensure_managed_target(manifest, &codex_home.join("config.toml"))?;
        ensure_managed_target(manifest, &codex_home.join("auth.json"))?;
    }
    Ok(())
}

fn is_normalized_absolute(path: &Path) -> bool {
    path.is_absolute()
        && !path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
}

fn ensure_managed_target(manifest: &EffectiveRuntimeManifest, target: &Path) -> Result<()> {
    let allowed = match manifest.isolation {
        RuntimeIsolation::SharedAccountHome => {
            Path::new(&manifest.project.project_root).join(".agents/skills")
        }
        RuntimeIsolation::Hard => PathBuf::from(&manifest.runtime_root),
    };
    if !target.starts_with(&allowed) {
        bail!(
            "projection target {} escapes managed root {}",
            target.display(),
            allowed.display()
        );
    }
    Ok(())
}

fn projection_matches(source: &Path, target: &Path, mode: ProjectionMode) -> Result<bool> {
    if !target.exists() && !target.is_symlink() {
        return Ok(false);
    }
    match mode {
        ProjectionMode::Symlink => {
            if !target.is_symlink() {
                return Ok(false);
            }
            let linked = fs::read_link(target)
                .with_context(|| format!("failed to read projection link {}", target.display()))?;
            Ok(linked == source)
        }
        ProjectionMode::Copy => {
            if !target.is_dir() || !source.is_dir() {
                return Ok(false);
            }
            let marker = target.join(COPY_OWNERSHIP_MARKER);
            let contents = match fs::read(&marker) {
                Ok(contents) => contents,
                Err(_) => return Ok(false),
            };
            let value: serde_json::Value = serde_json::from_slice(&contents)
                .with_context(|| format!("invalid projection marker {}", marker.display()))?;
            Ok(value.get("sourcePath").and_then(|item| item.as_str())
                == Some(source.to_string_lossy().as_ref()))
        }
    }
}

fn verify_entries(entries: &[ProjectionEntry]) -> Result<bool> {
    for entry in entries {
        if entry.operation == ProjectionOperation::SkipMissingOptional {
            continue;
        }
        if !projection_matches(
            Path::new(&entry.source_path),
            Path::new(&entry.target_path),
            entry.mode,
        )? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn remove_projection_target(path: &Path) -> Result<()> {
    if path.is_symlink() || path.is_file() {
        fs::remove_file(path)
            .with_context(|| format!("failed to remove projection {}", path.display()))?;
    } else if path.is_dir() {
        fs::remove_dir_all(path)
            .with_context(|| format!("failed to remove projection {}", path.display()))?;
    }
    Ok(())
}

fn projection_backup_path(target: &Path, index: usize) -> Result<PathBuf> {
    let parent = target
        .parent()
        .context("projection target has no parent directory")?;
    let name = target
        .file_name()
        .and_then(|value| value.to_str())
        .context("projection target has no UTF-8 file name")?;
    let backup = parent.join(format!(
        ".codexhome-backup-{name}-{}-{index}",
        std::process::id()
    ));
    if backup.exists() || backup.is_symlink() {
        bail!(
            "projection backup path already exists: {}",
            backup.display()
        );
    }
    Ok(backup)
}

#[cfg(unix)]
fn create_symlink(source: &Path, target: &Path) -> Result<()> {
    std::os::unix::fs::symlink(source, target).with_context(|| {
        format!(
            "failed to link projection {} -> {}",
            target.display(),
            source.display()
        )
    })
}

#[cfg(windows)]
fn create_symlink(source: &Path, target: &Path) -> Result<()> {
    let result = if source.is_dir() {
        std::os::windows::fs::symlink_dir(source, target)
    } else {
        std::os::windows::fs::symlink_file(source, target)
    };
    result.with_context(|| {
        format!(
            "failed to link projection {} -> {}",
            target.display(),
            source.display()
        )
    })
}

fn copy_tree(source: &Path, target: &Path) -> Result<()> {
    if source.is_file() {
        fs::copy(source, target).with_context(|| {
            format!(
                "failed to copy projection {} -> {}",
                source.display(),
                target.display()
            )
        })?;
        return Ok(());
    }
    if !source.is_dir() {
        bail!("projection source is not a file or directory");
    }
    fs::create_dir_all(target)
        .with_context(|| format!("failed to create projection {}", target.display()))?;
    for item in fs::read_dir(source)
        .with_context(|| format!("failed to read projection source {}", source.display()))?
    {
        let item = item?;
        let child_source = item.path();
        let child_target = target.join(item.file_name());
        let file_type = item.file_type()?;
        if file_type.is_symlink() {
            bail!(
                "copy projection refuses nested symlink {}",
                child_source.display()
            );
        }
        if file_type.is_dir() {
            copy_tree(&child_source, &child_target)?;
        } else if file_type.is_file() {
            fs::copy(&child_source, &child_target)?;
        }
    }
    let marker = serde_json::to_vec_pretty(&serde_json::json!({
        "schemaVersion": RUNTIME_PROJECTION_SCHEMA_VERSION,
        "sourcePath": source.to_string_lossy(),
    }))
    .context("failed to encode projection ownership marker")?;
    fs::write(target.join(COPY_OWNERSHIP_MARKER), marker)
        .context("failed to write projection ownership marker")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ContextFootprint, EffectiveAccountLayer, EffectiveProjectLayer, EffectiveSkill,
        EFFECTIVE_RUNTIME_SCHEMA_VERSION,
    };
    use tempfile::TempDir;

    fn manifest(temp: &TempDir) -> EffectiveRuntimeManifest {
        let account = temp.path().join("account");
        let runtime = temp.path().join("runtime");
        let source = temp.path().join("source/skill");
        fs::create_dir_all(&account).expect("account");
        fs::create_dir_all(&source).expect("skill");
        fs::write(account.join("config.toml"), "model = \"test\"").expect("config");
        fs::write(account.join("auth.json"), "{}").expect("auth");
        fs::write(source.join("SKILL.md"), "# Test").expect("skill file");
        EffectiveRuntimeManifest {
            ok: true,
            schema_version: EFFECTIVE_RUNTIME_SCHEMA_VERSION.to_owned(),
            manifest_id: "runtime-test".to_owned(),
            account: EffectiveAccountLayer {
                id: "account-main".to_owned(),
                label: "Main".to_owned(),
                home_path: account.to_string_lossy().into_owned(),
                provider: None,
                model: None,
                endpoint_host: None,
                has_auth: true,
                has_config: true,
            },
            experts: Vec::new(),
            project: EffectiveProjectLayer {
                id: "binding-test".to_owned(),
                project_id: "project-test".to_owned(),
                project_root: temp.path().join("project").to_string_lossy().into_owned(),
                has_project_home: false,
            },
            isolation: RuntimeIsolation::Hard,
            runtime_root: runtime.to_string_lossy().into_owned(),
            codex_home: runtime.to_string_lossy().into_owned(),
            skills: vec![EffectiveSkill {
                id: "test-skill".to_owned(),
                label: "Test".to_owned(),
                version: "1.0.0".to_owned(),
                source_path: source.to_string_lossy().into_owned(),
                target_path: runtime
                    .join("skills/test-skill")
                    .to_string_lossy()
                    .into_owned(),
                digest_sha256: compute_skill_digest(&source).expect("skill digest"),
                required: true,
            }],
            agents_files: Vec::new(),
            rule_files: Vec::new(),
            mcp_server_ids: Vec::new(),
            context: ContextFootprint::default(),
            safety: SafetySummary {
                secrets_redacted: true,
                includes_secrets: false,
                reads_auth_contents: false,
                includes_local_paths: true,
            },
        }
    }

    #[test]
    fn hard_projection_links_account_and_skill_without_reading_auth() {
        let temp = TempDir::new().expect("temp");
        let manifest = manifest(&temp);
        let report = RuntimeProjection::apply(&manifest, ProjectionMode::Symlink, false, false)
            .expect("apply projection");
        assert!(report.ok);
        assert!(report.verified);
        assert!(Path::new(&manifest.codex_home)
            .join("auth.json")
            .is_symlink());
        assert!(Path::new(&manifest.skills[0].target_path).is_symlink());

        let clean = RuntimeProjection::clean(&manifest, ProjectionMode::Symlink, false)
            .expect("clean projection");
        assert!(clean.ok);
        assert!(!Path::new(&manifest.skills[0].target_path).exists());
        assert!(Path::new(&manifest.account.home_path)
            .join("auth.json")
            .exists());
    }

    #[test]
    fn apply_and_clean_refuse_unowned_existing_targets() {
        let temp = TempDir::new().expect("temp");
        let manifest = manifest(&temp);
        let target = Path::new(&manifest.skills[0].target_path);
        fs::create_dir_all(target).expect("unowned target");
        fs::write(target.join("user-file.txt"), "preserve").expect("user file");

        let apply_error =
            RuntimeProjection::apply(&manifest, ProjectionMode::Symlink, false, false)
                .expect_err("replace requires force");
        assert!(apply_error.to_string().contains("pass force"));
        assert!(target.join("user-file.txt").exists());

        let clean_error = RuntimeProjection::clean(&manifest, ProjectionMode::Symlink, false)
            .expect_err("clean requires ownership");
        assert!(clean_error.to_string().contains("unowned"));
        assert!(target.join("user-file.txt").exists());
    }

    #[test]
    fn plan_rejects_skill_source_drift() {
        let temp = TempDir::new().expect("temp");
        let manifest = manifest(&temp);
        fs::write(
            Path::new(&manifest.skills[0].source_path).join("SKILL.md"),
            "# Changed after registry digest",
        )
        .expect("change skill");
        let error = RuntimeProjection::plan(&manifest, ProjectionMode::Symlink)
            .expect_err("digest drift must fail");
        assert!(error.to_string().contains("digest mismatch"));
    }

    #[test]
    fn hard_projection_rejects_codex_home_outside_runtime_root() {
        let temp = TempDir::new().expect("temp");
        let mut manifest = manifest(&temp);
        manifest.codex_home = temp
            .path()
            .join("outside-runtime")
            .to_string_lossy()
            .into_owned();
        let error = RuntimeProjection::plan(&manifest, ProjectionMode::Symlink)
            .expect_err("hard runtime escape");
        assert!(error.to_string().contains("must equal"));
    }
}
