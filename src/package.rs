//! File changes shared by installation, update and repair.
use anyhow::{Context, Result, bail};
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::{NamedTempFile, TempPath};

#[derive(Default)]
pub struct FileTransaction {
    changes: Vec<(PathBuf, Option<TempPath>)>,
    committed: bool,
}

impl FileTransaction {
    pub fn track(&mut self, path: &Path) -> Result<()> {
        if self.changes.iter().any(|(target, _)| target == path) {
            return Ok(());
        }
        let parent = path.parent().context("arquivo sem diretório")?;
        std::fs::create_dir_all(parent)?;
        let backup = if path.exists() {
            let mut backup = NamedTempFile::new_in(parent)?;
            let mut source = std::fs::File::open(path)?;
            std::io::copy(&mut source, &mut backup)?;
            backup.as_file().sync_all()?;
            Some(backup.into_temp_path())
        } else {
            None
        };
        self.changes.push((path.to_owned(), backup));
        Ok(())
    }

    pub fn replace(&mut self, path: &Path, bytes: &[u8]) -> Result<()> {
        self.track(path)?;
        let mut staged = NamedTempFile::new_in(path.parent().context("arquivo sem diretório")?)?;
        staged.write_all(bytes)?;
        staged.as_file().sync_all()?;
        staged.persist(path).map_err(|error| error.error)?;
        Ok(())
    }

    pub fn commit(mut self) {
        self.committed = true;
    }

    pub fn rollback(&mut self) -> Result<()> {
        let mut failures = Vec::new();
        while let Some((target, backup)) = self.changes.pop() {
            if let Some(backup) = backup {
                if let Err(error) = backup.persist(&target) {
                    let reason = error.error.to_string();
                    // Keep the old bytes if the OS refuses to restore them.
                    let saved = error.path.keep().ok();
                    failures.push(format!(
                        "{}: {reason}; cópia anterior: {saved:?}",
                        target.display()
                    ));
                }
            } else if target.exists()
                && let Err(error) = std::fs::remove_file(&target)
            {
                failures.push(format!("{}: {error}", target.display()));
            }
        }
        self.committed = true;
        if !failures.is_empty() {
            bail!(
                "não foi possível restaurar todos os arquivos: {}",
                failures.join("; ")
            );
        }
        Ok(())
    }
}

impl Drop for FileTransaction {
    fn drop(&mut self) {
        if !self.committed {
            let _ = self.rollback();
        }
    }
}

/// Move an in-use file out of its installation name before scheduling removal.
/// A later reinstall must never inherit a deletion queued for the old path.
pub fn retire_file(path: &Path) -> Result<PathBuf> {
    let retired = tempfile::Builder::new()
        .prefix("BloqueioTransparente-pending-delete-")
        .tempfile_in(path.parent().context("arquivo sem diretório")?)?
        .into_temp_path();
    std::fs::rename(path, &retired)
        .context("não foi possível separar o arquivo em uso para remoção")?;
    Ok(retired.keep()?)
}

pub fn version_numbers(version: &str) -> Option<Vec<u32>> {
    let mut parts = version
        .split('.')
        .map(str::parse::<u32>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    if !(3..=4).contains(&parts.len()) {
        return None;
    }
    parts.resize(4, 0);
    Some(parts)
}

pub fn is_downgrade(installed: &str, incoming: &str) -> bool {
    match (version_numbers(installed), version_numbers(incoming)) {
        (Some(installed), Some(incoming)) => installed > incoming,
        _ => false,
    }
}
