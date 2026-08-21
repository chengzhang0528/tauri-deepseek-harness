use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use directories::BaseDirs;

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub local_root: PathBuf,
    pub roaming_root: PathBuf,
    pub launcher: PathBuf,
    pub runtimes: PathBuf,
    pub cache: PathBuf,
    pub staging: PathBuf,
    pub state: PathBuf,
    pub logs: PathBuf,
    pub dsh_home: PathBuf,
}

impl AppPaths {
    pub fn discover() -> Result<Self> {
        let base = BaseDirs::new().context("cannot resolve Windows user directories")?;
        let local_root = base.data_local_dir().join("DSH Desktop");
        let roaming_root = base.data_dir().join("DSH Desktop");
        Ok(Self {
            launcher: local_root.join("launcher"),
            runtimes: local_root.join("runtimes"),
            cache: local_root.join("cache"),
            staging: local_root.join("staging"),
            state: local_root.join("state"),
            logs: local_root.join("logs"),
            dsh_home: roaming_root.join("dsh-home"),
            local_root,
            roaming_root,
        })
    }

    pub fn create(&self) -> Result<()> {
        for path in [
            &self.local_root,
            &self.roaming_root,
            &self.launcher,
            &self.runtimes,
            &self.cache,
            &self.staging,
            &self.state,
            &self.logs,
            &self.dsh_home,
        ] {
            fs::create_dir_all(path)
                .with_context(|| format!("cannot create {}", path.display()))?;
        }
        Ok(())
    }

    pub fn remove_installer_owned_state(&self) -> Result<()> {
        for path in [
            &self.launcher,
            &self.runtimes,
            &self.cache,
            &self.staging,
            &self.state,
            &self.logs,
        ] {
            if path.exists() {
                fs::remove_dir_all(path)
                    .with_context(|| format!("cannot remove {}", path.display()))?;
            }
        }
        Ok(())
    }

    pub fn current_pointer(&self) -> PathBuf {
        self.state.join("current.json")
    }

    pub fn staged_pointer(&self) -> PathBuf {
        self.state.join("staged.json")
    }

    pub fn repair_record(&self) -> PathBuf {
        self.state.join("repair.json")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn removes_only_installer_owned_local_state() {
        let root = tempdir().expect("temp");
        let paths = AppPaths {
            local_root: root.path().join("local"),
            roaming_root: root.path().join("roaming"),
            launcher: root.path().join("local/launcher"),
            runtimes: root.path().join("local/runtimes"),
            cache: root.path().join("local/cache"),
            staging: root.path().join("local/staging"),
            state: root.path().join("local/state"),
            logs: root.path().join("local/logs"),
            dsh_home: root.path().join("roaming/dsh-home"),
        };
        paths.create().expect("create state");
        fs::write(paths.cache.join("asset"), b"cache").expect("cache");
        fs::write(paths.dsh_home.join("settings"), b"keep").expect("home");

        paths
            .remove_installer_owned_state()
            .expect("remove local state");

        assert!(!paths.cache.exists());
        assert!(!paths.state.exists());
        assert!(paths.dsh_home.join("settings").exists());
        assert!(paths.roaming_root.exists());
    }
}
