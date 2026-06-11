use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Clone)]
pub struct LauncherPaths {
    pub root_dir: PathBuf,
    pub instance_dir: PathBuf,
    pub cache_dir: PathBuf,
}

#[derive(Debug, Error)]
pub enum LauncherPathError {
    #[error("could not locate a user data directory")]
    MissingDataDirectory,
    #[error("failed to create launcher directory: {0}")]
    CreateDirectory(#[from] std::io::Error),
}

impl LauncherPaths {
    pub fn discover() -> Result<Self, LauncherPathError> {
        let base = dirs::data_dir().ok_or(LauncherPathError::MissingDataDirectory)?;
        let root_dir = base.join("TownRiseLauncher");
        let instance_dir = root_dir.join("instance");
        let cache_dir = root_dir.join("cache");
        std::fs::create_dir_all(&instance_dir)?;
        std::fs::create_dir_all(&cache_dir)?;
        Ok(Self {
            root_dir,
            instance_dir,
            cache_dir,
        })
    }
}
