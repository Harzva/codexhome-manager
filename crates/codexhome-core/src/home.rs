use crate::registry::{normalize_absolute_path, RegistryEntry, RegistryOrigin, RegistryStore};
use crate::{inspect_home_path, CodexHomeSummary};
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

pub const HOME_MUTATION_SCHEMA_VERSION: &str = "codexhome.home-mutation.v1";
const CAPABILITY_DIRECTORIES: [&str; 3] = ["skills", "rules", "hooks"];
const MAX_CAPABILITY_FILES: usize = 20_000;
const MAX_CAPABILITY_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CopySummary {
    pub files_copied: usize,
    pub directories_created: usize,
    pub files_skipped: usize,
    pub bytes_copied: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HomeMutationResult {
    pub ok: bool,
    pub schema_version: &'static str,
    pub action: &'static str,
    pub dry_run: bool,
    pub registry_revision: u64,
    pub entry: RegistryEntry,
    pub copy_summary: CopySummary,
    pub planned_actions: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct HomeManager {
    store: RegistryStore,
}

impl HomeManager {
    pub fn new(store: RegistryStore) -> Self {
        Self { store }
    }

    pub fn create(
        &self,
        alias: &str,
        label: Option<&str>,
        target: &Path,
        specialties: Vec<String>,
        dry_run: bool,
    ) -> Result<HomeMutationResult> {
        let target = normalize_absolute_path(target)?;
        ensure_new_target(&target)?;
        let entry = RegistryEntry::new(
            alias,
            label,
            &target,
            specialties,
            RegistryOrigin::Created,
            None,
        )?;
        self.store.preview_add(&entry)?;
        let planned_actions = vec![
            format!("create Home directory {}", target.display()),
            "create config.toml and empty skills/rules/hooks directories".to_owned(),
            format!("register {}", entry.alias),
        ];
        if dry_run {
            return self.preview("create", entry, planned_actions, Vec::new());
        }

        if let Err(error) = create_skeleton(&target, None) {
            rollback_new_home(&target);
            return Err(error).context("failed to create Home skeleton");
        }
        let registry = match self.store.add(entry.clone()) {
            Ok(registry) => registry,
            Err(error) => {
                rollback_new_home(&target);
                return Err(error)
                    .context("failed to register new Home; created files were rolled back");
            }
        };
        Ok(HomeMutationResult {
            ok: true,
            schema_version: HOME_MUTATION_SCHEMA_VERSION,
            action: "create",
            dry_run: false,
            registry_revision: registry.revision,
            entry,
            copy_summary: CopySummary::default(),
            planned_actions,
            warnings: vec![
                "Authentication was not created; sign in or configure the provider inside this Home before use."
                    .to_owned(),
            ],
        })
    }

    pub fn import(
        &self,
        source: &Path,
        alias: &str,
        label: Option<&str>,
        specialties: Vec<String>,
        dry_run: bool,
    ) -> Result<HomeMutationResult> {
        let source = normalize_absolute_path(source)?;
        let (summary, _) = inspect_home_path(&source, "imported")?;
        let entry = RegistryEntry::new(
            alias,
            label,
            &source,
            specialties,
            RegistryOrigin::Imported,
            None,
        )?;
        self.store.preview_add(&entry)?;
        let planned_actions = vec![
            format!("validate existing Home {}", source.display()),
            format!("register {} without changing its files", entry.alias),
        ];
        let mut warnings = Vec::new();
        if !summary.has_auth {
            warnings.push("The imported Home has no auth.json file.".to_owned());
        }
        if dry_run {
            return self.preview("import", entry, planned_actions, warnings);
        }
        let registry = self.store.add(entry.clone())?;
        Ok(HomeMutationResult {
            ok: true,
            schema_version: HOME_MUTATION_SCHEMA_VERSION,
            action: "import",
            dry_run: false,
            registry_revision: registry.revision,
            entry,
            copy_summary: CopySummary::default(),
            planned_actions,
            warnings,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn clone_home(
        &self,
        source_query: &str,
        alias: &str,
        label: Option<&str>,
        target: &Path,
        specialties: Vec<String>,
        copy_capabilities: bool,
        dry_run: bool,
    ) -> Result<HomeMutationResult> {
        let source_entry = self.store.resolve(source_query)?;
        let source = normalize_absolute_path(&source_entry.path_buf())?;
        let target = normalize_absolute_path(target)?;
        ensure_new_target(&target)?;
        if target.starts_with(&source) {
            bail!("clone destination cannot be inside the source Home");
        }
        let (source_summary, _) = inspect_home_path(&source, "clone-source")?;
        let entry = RegistryEntry::new(
            alias,
            label,
            &target,
            specialties,
            RegistryOrigin::Cloned,
            Some(source_entry.id.clone()),
        )?;
        self.store.preview_add(&entry)?;
        let mut planned_actions = vec![
            format!("create Home directory {}", target.display()),
            "copy allowlisted non-provider configuration".to_owned(),
            "exclude authentication, sessions, databases, logs, plugins, and provider credentials"
                .to_owned(),
        ];
        if copy_capabilities {
            planned_actions.push(
                "copy Skills, Rules, and Hooks while skipping symlinks and secret filenames"
                    .to_owned(),
            );
        } else {
            planned_actions.push("create empty Skills, Rules, and Hooks directories".to_owned());
        }
        planned_actions.push(format!("register {}", entry.alias));

        let copy_summary = if copy_capabilities {
            copy_capability_trees(&source, &target, false)?
        } else {
            CopySummary::default()
        };
        let mut warnings = vec![
            "Authentication and provider configuration are never cloned; configure them separately in the new Home."
                .to_owned(),
        ];
        if !copy_capabilities {
            warnings.push(
                "Capabilities were not copied; pass --copy-capabilities after reviewing the source Home."
                    .to_owned(),
            );
        }
        if dry_run {
            return Ok(HomeMutationResult {
                ok: true,
                schema_version: HOME_MUTATION_SCHEMA_VERSION,
                action: "clone",
                dry_run: true,
                registry_revision: self.store.load()?.revision,
                entry,
                copy_summary,
                planned_actions,
                warnings,
            });
        }

        if let Err(error) = create_skeleton(&target, Some(&source_summary)) {
            rollback_new_home(&target);
            return Err(error).context("failed to create clone skeleton");
        }
        let copy_summary = if copy_capabilities {
            match copy_capability_trees(&source, &target, true) {
                Ok(summary) => summary,
                Err(error) => {
                    rollback_new_home(&target);
                    return Err(error)
                        .context("failed to copy capabilities; clone was rolled back");
                }
            }
        } else {
            CopySummary::default()
        };
        let registry = match self.store.add(entry.clone()) {
            Ok(registry) => registry,
            Err(error) => {
                rollback_new_home(&target);
                return Err(error)
                    .context("failed to register clone; cloned files were rolled back");
            }
        };
        Ok(HomeMutationResult {
            ok: true,
            schema_version: HOME_MUTATION_SCHEMA_VERSION,
            action: "clone",
            dry_run: false,
            registry_revision: registry.revision,
            entry,
            copy_summary,
            planned_actions,
            warnings,
        })
    }

    fn preview(
        &self,
        action: &'static str,
        entry: RegistryEntry,
        planned_actions: Vec<String>,
        warnings: Vec<String>,
    ) -> Result<HomeMutationResult> {
        Ok(HomeMutationResult {
            ok: true,
            schema_version: HOME_MUTATION_SCHEMA_VERSION,
            action,
            dry_run: true,
            registry_revision: self.store.load()?.revision,
            entry,
            copy_summary: CopySummary::default(),
            planned_actions,
            warnings,
        })
    }
}

fn ensure_new_target(path: &Path) -> Result<()> {
    if path.exists() {
        bail!(
            "clone/create destination already exists: {}",
            path.display()
        );
    }
    let parent = path
        .parent()
        .filter(|value| !value.as_os_str().is_empty())
        .context("Home destination must have a parent directory")?;
    if parent.exists() && !parent.is_dir() {
        bail!(
            "Home destination parent is not a directory: {}",
            parent.display()
        );
    }
    Ok(())
}

fn create_skeleton(path: &Path, source: Option<&CodexHomeSummary>) -> Result<()> {
    fs::create_dir_all(path)
        .with_context(|| format!("failed to create Home directory {}", path.display()))?;
    for name in CAPABILITY_DIRECTORIES {
        fs::create_dir(path.join(name))
            .with_context(|| format!("failed to create {name} directory"))?;
    }
    let config = safe_config(source)?;
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path.join("config.toml"))
        .context("failed to create config.toml")?;
    file.write_all(config.as_bytes())
        .context("failed to write config.toml")?;
    file.sync_all().context("failed to sync config.toml")?;
    Ok(())
}

fn safe_config(source: Option<&CodexHomeSummary>) -> Result<String> {
    let mut output = String::from(
        "# Generated by CodexHome Manager. Authentication and provider credentials are intentionally absent.\n",
    );
    let Some(source) = source else {
        return Ok(output);
    };
    let mut table = toml::Table::new();
    insert_string(&mut table, "model", source.model.as_deref());
    insert_string(
        &mut table,
        "model_reasoning_effort",
        source.reasoning_effort.as_deref(),
    );
    insert_string(
        &mut table,
        "approval_policy",
        source.approval_policy.as_deref(),
    );
    insert_string(&mut table, "sandbox_mode", source.sandbox_mode.as_deref());
    if let Some(value) = source.web_search.as_deref() {
        let value = match value {
            "true" => toml::Value::Boolean(true),
            "false" => toml::Value::Boolean(false),
            other => toml::Value::String(other.to_owned()),
        };
        table.insert("web_search".to_owned(), value);
    }
    output.push_str(&toml::to_string_pretty(&table).context("failed to build safe config")?);
    Ok(output)
}

fn insert_string(table: &mut toml::Table, key: &str, value: Option<&str>) {
    if let Some(value) = value {
        table.insert(key.to_owned(), toml::Value::String(value.to_owned()));
    }
}

fn copy_capability_trees(source: &Path, target: &Path, write: bool) -> Result<CopySummary> {
    let mut summary = CopySummary::default();
    for name in CAPABILITY_DIRECTORIES {
        let source_root = source.join(name);
        let Ok(metadata) = fs::symlink_metadata(&source_root) else {
            continue;
        };
        if metadata.file_type().is_symlink() {
            summary.files_skipped += 1;
            continue;
        }
        if !metadata.is_dir() {
            continue;
        }
        let target_root = target.join(name);
        visit_capability_tree(&source_root, &target_root, write, &mut summary)?;
    }
    Ok(summary)
}

fn visit_capability_tree(
    source: &Path,
    target: &Path,
    write: bool,
    summary: &mut CopySummary,
) -> Result<()> {
    let entries = fs::read_dir(source).with_context(|| {
        format!(
            "failed to inspect capability directory {}",
            source.display()
        )
    })?;
    for entry in entries {
        let entry = entry.with_context(|| format!("failed to inspect {}", source.display()))?;
        let file_type = entry
            .file_type()
            .context("failed to inspect capability file type")?;
        let name = entry.file_name();
        let source_path = entry.path();
        let target_path = target.join(&name);
        if file_type.is_symlink() || is_sensitive_name(&name.to_string_lossy()) {
            summary.files_skipped += 1;
            continue;
        }
        if file_type.is_dir() {
            summary.directories_created += 1;
            if write {
                fs::create_dir_all(&target_path).with_context(|| {
                    format!(
                        "failed to create capability directory {}",
                        target_path.display()
                    )
                })?;
            }
            visit_capability_tree(&source_path, &target_path, write, summary)?;
        } else if file_type.is_file() {
            let bytes = entry
                .metadata()
                .context("failed to inspect capability file")?
                .len();
            summary.files_copied += 1;
            summary.bytes_copied = summary
                .bytes_copied
                .checked_add(bytes)
                .context("capability size overflow")?;
            if summary.files_copied > MAX_CAPABILITY_FILES
                || summary.bytes_copied > MAX_CAPABILITY_BYTES
            {
                bail!(
                    "capability clone exceeds the safety limit of {MAX_CAPABILITY_FILES} files or {} MiB",
                    MAX_CAPABILITY_BYTES / 1024 / 1024
                );
            }
            if write {
                fs::copy(&source_path, &target_path).with_context(|| {
                    format!("failed to copy capability file {}", source_path.display())
                })?;
            }
        } else {
            summary.files_skipped += 1;
        }
    }
    Ok(())
}

fn is_sensitive_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name == ".git"
        || name == ".env"
        || name.starts_with(".env.")
        || name.ends_with(".pem")
        || name.ends_with(".key")
        || matches!(
            name.as_str(),
            "auth.json"
                | "credentials"
                | "credentials.json"
                | "secret.json"
                | "secrets.json"
                | "token.json"
                | "tokens.json"
                | "cookies.json"
                | ".netrc"
                | ".npmrc"
                | ".pypirc"
                | "id_rsa"
                | "id_ed25519"
        )
}

fn rollback_new_home(path: &Path) {
    if path.exists() {
        let _ = fs::remove_dir_all(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn create_makes_a_registered_empty_home() {
        let temp = TempDir::new().expect("temp");
        let store = RegistryStore::new(temp.path().join("registry.json"));
        let manager = HomeManager::new(store.clone());
        let target = temp.path().join("frontend");
        let result = manager
            .create(
                "@frontend",
                Some("Frontend Family"),
                &target,
                vec!["ui".into(), "前端".into()],
                false,
            )
            .expect("create");
        assert!(!result.dry_run);
        assert!(target.join("config.toml").is_file());
        assert!(target.join("skills").is_dir());
        assert!(!target.join("auth.json").exists());
        assert_eq!(
            store.resolve("@frontend").expect("registered").label,
            "Frontend Family"
        );
    }

    #[test]
    fn import_does_not_modify_the_existing_home() {
        let temp = TempDir::new().expect("temp");
        let home = temp.path().join("research");
        fs::create_dir(&home).expect("home");
        fs::write(home.join("config.toml"), "model = \"gpt-test\"\n").expect("config");
        let store = RegistryStore::new(temp.path().join("registry.json"));
        let manager = HomeManager::new(store.clone());
        manager
            .import(&home, "@research", None, vec!["papers".into()], false)
            .expect("import");
        assert_eq!(
            fs::read_to_string(home.join("config.toml")).expect("read"),
            "model = \"gpt-test\"\n"
        );
        assert_eq!(
            store.resolve("research").expect("resolve").origin,
            RegistryOrigin::Imported
        );
    }

    #[test]
    fn clone_excludes_credentials_and_copies_capabilities_only_when_requested() {
        let temp = TempDir::new().expect("temp");
        let source = temp.path().join("source");
        fs::create_dir_all(source.join("skills/web")).expect("source");
        fs::write(
            source.join("config.toml"),
            "model = \"gpt-test\"\napi_key = \"never-copy\"\n",
        )
        .expect("config");
        fs::write(source.join("auth.json"), "never-copy-auth").expect("auth");
        fs::write(source.join("skills/web/SKILL.md"), "safe capability").expect("skill");
        fs::write(source.join("skills/web/.env"), "never-copy-env").expect("env");

        let store = RegistryStore::new(temp.path().join("registry.json"));
        let manager = HomeManager::new(store.clone());
        manager
            .import(&source, "@source", None, vec![], false)
            .expect("import");
        let target = temp.path().join("clone");
        let result = manager
            .clone_home(
                "@source",
                "@clone",
                None,
                &target,
                vec!["web".into()],
                true,
                false,
            )
            .expect("clone");
        let config = fs::read_to_string(target.join("config.toml")).expect("clone config");
        assert!(config.contains("gpt-test"));
        assert!(!config.contains("never-copy"));
        assert!(!target.join("auth.json").exists());
        assert!(target.join("skills/web/SKILL.md").is_file());
        assert!(!target.join("skills/web/.env").exists());
        assert_eq!(result.copy_summary.files_copied, 1);
        assert_eq!(result.copy_summary.files_skipped, 1);
    }

    #[test]
    fn dry_run_changes_neither_home_nor_registry() {
        let temp = TempDir::new().expect("temp");
        let store = RegistryStore::new(temp.path().join("registry.json"));
        let manager = HomeManager::new(store.clone());
        let target = temp.path().join("planned");
        let result = manager
            .create("@planned", None, &target, vec![], true)
            .expect("dry run");
        assert!(result.dry_run);
        assert!(!target.exists());
        assert!(!store.path().exists());
    }

    #[test]
    fn create_rolls_back_when_registry_commit_fails() {
        let temp = TempDir::new().expect("temp");
        let registry_parent = temp.path().join("not-a-directory");
        fs::write(&registry_parent, "block registry directory").expect("blocker");
        let store = RegistryStore::new(registry_parent.join("registry.json"));
        let manager = HomeManager::new(store);
        let target = temp.path().join("rolled-back");

        let error = manager
            .create("@rollback", None, &target, vec![], false)
            .expect_err("registry commit must fail");

        assert!(error.to_string().contains("rolled back"));
        assert!(!target.exists());
    }
}
