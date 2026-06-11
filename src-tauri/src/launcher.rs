use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    path::{Component, Path, PathBuf},
    process::Command,
};

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

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MinecraftLaunchConfig {
    pub java_executable: Option<String>,
    pub working_directory: Option<String>,
    #[serde(default)]
    pub jvm_args: Vec<String>,
    pub classpath: Vec<String>,
    pub main_class: String,
    #[serde(default)]
    pub game_args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MinecraftCommand {
    pub program: String,
    pub args: Vec<String>,
    pub working_directory: PathBuf,
}

#[derive(Debug, Error)]
pub enum LaunchError {
    #[error("Minecraft launch config is missing. Expected {0}")]
    MissingConfig(PathBuf),
    #[error("Minecraft launch config is invalid: {0}")]
    InvalidConfig(String),
    #[error("failed to read Minecraft launch config: {0}")]
    ReadConfig(#[from] std::io::Error),
    #[error("failed to parse Minecraft launch config: {0}")]
    ParseConfig(#[from] serde_json::Error),
    #[error("failed to start Minecraft process: {0}")]
    Spawn(std::io::Error),
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

    pub fn launch_config_path(&self) -> PathBuf {
        self.instance_dir.join("launch.json")
    }
}

pub fn read_launch_config(path: &Path) -> Result<MinecraftLaunchConfig, LaunchError> {
    if !path.exists() {
        return Err(LaunchError::MissingConfig(path.to_path_buf()));
    }
    let raw = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&raw)?)
}

pub fn build_minecraft_command(
    config: &MinecraftLaunchConfig,
    instance_dir: &Path,
) -> Result<MinecraftCommand, LaunchError> {
    if config.classpath.is_empty() {
        return Err(LaunchError::InvalidConfig(
            "classpath must contain at least one jar".into(),
        ));
    }
    if config.main_class.trim().is_empty() || config.main_class.contains(char::is_whitespace) {
        return Err(LaunchError::InvalidConfig(
            "mainClass must be a Java class name".into(),
        ));
    }

    let working_directory = match config.working_directory.as_deref() {
        Some(raw) => safe_instance_path(instance_dir, raw)?,
        None => instance_dir.to_path_buf(),
    };
    let classpath = config
        .classpath
        .iter()
        .map(|entry| safe_instance_path(instance_dir, entry))
        .collect::<Result<Vec<_>, _>>()?;
    let joined_classpath = std::env::join_paths(classpath)
        .map_err(|error| LaunchError::InvalidConfig(format!("invalid classpath: {error}")))?;

    let mut args = Vec::new();
    args.extend(config.jvm_args.clone());
    args.push("-cp".into());
    args.push(os_to_string(joined_classpath));
    args.push(config.main_class.clone());
    args.extend(config.game_args.clone());

    Ok(MinecraftCommand {
        program: java_program(config, instance_dir)?,
        args,
        working_directory,
    })
}

fn java_program(
    config: &MinecraftLaunchConfig,
    instance_dir: &Path,
) -> Result<String, LaunchError> {
    let Some(raw) = &config.java_executable else {
        return Ok("java".to_string());
    };
    if raw.trim().is_empty() {
        return Ok("java".to_string());
    }
    if raw == "java" || raw == "java.exe" {
        return Ok(raw.clone());
    }
    let resolved = safe_instance_path(instance_dir, raw)?;
    Ok(os_to_string(resolved.into_os_string()))
}

pub fn launch_minecraft(paths: &LauncherPaths) -> Result<u32, LaunchError> {
    let config = read_launch_config(&paths.launch_config_path())?;
    let command = build_minecraft_command(&config, &paths.instance_dir)?;
    let child = Command::new(&command.program)
        .args(&command.args)
        .current_dir(&command.working_directory)
        .spawn()
        .map_err(LaunchError::Spawn)?;
    Ok(child.id())
}

fn safe_instance_path(instance_dir: &Path, raw: &str) -> Result<PathBuf, LaunchError> {
    if raw.trim().is_empty() || raw.contains('\0') || raw.contains(':') {
        return Err(LaunchError::InvalidConfig(format!(
            "unsafe instance-relative path: {raw}"
        )));
    }
    let path = Path::new(raw);
    if path.is_absolute() {
        return Err(LaunchError::InvalidConfig(format!(
            "absolute paths are not allowed: {raw}"
        )));
    }

    let mut cleaned = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => cleaned.push(part),
            _ => {
                return Err(LaunchError::InvalidConfig(format!(
                    "path escapes the instance directory: {raw}"
                )))
            }
        }
    }

    Ok(instance_dir.join(cleaned))
}

fn os_to_string(value: OsString) -> String {
    value.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_config_rejects_paths_outside_instance() {
        let instance = PathBuf::from("/tmp/townrise-instance");
        let config = MinecraftLaunchConfig {
            java_executable: None,
            working_directory: None,
            jvm_args: vec![],
            classpath: vec!["../evil.jar".into()],
            main_class: "net.minecraft.client.main.Main".into(),
            game_args: vec![],
        };

        assert!(build_minecraft_command(&config, &instance).is_err());
    }

    #[test]
    fn launch_config_builds_java_command_with_instance_relative_classpath() {
        let instance = PathBuf::from("/tmp/townrise-instance");
        let config = MinecraftLaunchConfig {
            java_executable: Some("java".into()),
            working_directory: Some(".".into()),
            jvm_args: vec!["-Xmx4G".into()],
            classpath: vec![
                "libraries/a.jar".into(),
                "versions/townrise/client.jar".into(),
            ],
            main_class: "net.minecraft.client.main.Main".into(),
            game_args: vec!["--username".into(), "Player".into()],
        };

        let command = build_minecraft_command(&config, &instance).unwrap();

        assert_eq!(command.program, "java");
        assert_eq!(command.working_directory, instance);
        assert_eq!(command.args[0], "-Xmx4G");
        assert!(command.args.contains(&"-cp".to_string()));
        let classpath_arg = command
            .args
            .iter()
            .find(|arg| arg.contains("libraries") && arg.contains("versions"))
            .expect("classpath argument should include library and client jar paths");
        assert!(classpath_arg.contains(&format!("libraries{}a.jar", std::path::MAIN_SEPARATOR)));
        assert!(classpath_arg.contains(&format!(
            "versions{}townrise{}client.jar",
            std::path::MAIN_SEPARATOR,
            std::path::MAIN_SEPARATOR
        )));
        assert!(command
            .args
            .contains(&"net.minecraft.client.main.Main".to_string()));
        assert!(command.args.contains(&"--username".to_string()));
    }
}
