use crate::{
    auth::MinecraftSession,
    launcher::{LauncherPaths, MinecraftLaunchConfig},
};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::{collections::HashMap, io::Read, path::Path};
use thiserror::Error;

const MINECRAFT_VERSION: &str = "1.21.1";
const VERSION_MANIFEST_URL: &str =
    "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
const JAVA_RUNTIME_DIR: &str = "runtime/java-21";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapProgress {
    pub phase: String,
    pub current_file: Option<String>,
    pub completed_files: usize,
    pub total_files: usize,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub percent: u8,
}

#[derive(Debug, Error)]
pub enum BootstrapError {
    #[error("Minecraft metadata request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("Minecraft metadata is missing version 1.21.1")]
    MissingVersion,
    #[error("Minecraft bootstrap file operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("Minecraft metadata parse failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Minecraft download hash mismatch for {path}")]
    HashMismatch { path: String },
    #[error("Minecraft native extraction failed: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("Minecraft Java 21 runtime is unavailable for this platform")]
    UnsupportedJavaRuntime,
    #[error("Minecraft Java 21 runtime did not contain a java executable")]
    MissingJavaExecutable,
}

#[derive(Debug, Deserialize)]
struct VersionManifest {
    versions: Vec<VersionEntry>,
}

#[derive(Debug, Deserialize)]
struct VersionEntry {
    id: String,
    url: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VersionDetails {
    id: String,
    main_class: String,
    asset_index: AssetIndexInfo,
    downloads: ClientDownloads,
    libraries: Vec<Library>,
    #[serde(default)]
    arguments: Option<Arguments>,
    #[serde(default)]
    minecraft_arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AssetIndexInfo {
    id: String,
    url: String,
    sha1: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ClientDownloads {
    client: DownloadInfo,
}

#[derive(Debug, Deserialize, Clone)]
struct DownloadInfo {
    path: Option<String>,
    url: String,
    sha1: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Library {
    name: String,
    #[serde(default)]
    downloads: LibraryDownloads,
    #[serde(default)]
    natives: HashMap<String, String>,
    #[serde(default)]
    rules: Vec<Rule>,
}

#[derive(Debug, Deserialize, Default)]
struct LibraryDownloads {
    artifact: Option<DownloadInfo>,
    #[serde(default)]
    classifiers: HashMap<String, DownloadInfo>,
}

#[derive(Debug, Deserialize)]
struct Arguments {
    #[serde(default)]
    game: Vec<ArgumentValue>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ArgumentValue {
    Plain(String),
    Conditional {
        rules: Vec<Rule>,
        value: ArgumentPayload,
    },
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ArgumentPayload {
    One(String),
    Many(Vec<String>),
}

#[derive(Debug, Deserialize)]
struct Rule {
    action: String,
    os: Option<RuleOs>,
}

#[derive(Debug, Deserialize)]
struct RuleOs {
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AssetIndex {
    objects: HashMap<String, AssetObject>,
}

#[derive(Debug, Deserialize)]
struct AssetObject {
    hash: String,
    #[allow(dead_code)]
    size: u64,
}

struct ProgressReporter<F>
where
    F: FnMut(BootstrapProgress),
{
    callback: F,
    completed_files: usize,
    total_files: usize,
}

impl<F> ProgressReporter<F>
where
    F: FnMut(BootstrapProgress),
{
    fn new(callback: F) -> Self {
        Self {
            callback,
            completed_files: 0,
            total_files: 1,
        }
    }

    fn set_total(&mut self, total_files: usize) {
        self.total_files = total_files.max(1);
        self.emit("checking", None, 0, 0);
    }

    fn downloading(&mut self, current_file: impl Into<String>) {
        self.emit("downloading", Some(current_file.into()), 0, 0);
    }

    fn installed(&mut self, current_file: impl Into<String>) {
        self.completed_files = self.completed_files.saturating_add(1);
        self.emit("installed", Some(current_file.into()), 0, 0);
    }

    fn finished(&mut self) {
        self.completed_files = self.total_files;
        self.emit("finished", None, 0, 0);
    }

    fn emit(
        &mut self,
        phase: &str,
        current_file: Option<String>,
        downloaded_bytes: u64,
        total_bytes: u64,
    ) {
        let percent = ((self.completed_files.min(self.total_files) * 100) / self.total_files) as u8;
        (self.callback)(BootstrapProgress {
            phase: phase.into(),
            current_file,
            completed_files: self.completed_files,
            total_files: self.total_files,
            downloaded_bytes,
            total_bytes,
            percent,
        });
    }
}

pub async fn prepare_default_instance(
    paths: &LauncherPaths,
    session: &MinecraftSession,
) -> Result<(), BootstrapError> {
    prepare_default_instance_with_progress(paths, session, |_| {}).await
}

pub async fn prepare_default_instance_with_progress<F>(
    paths: &LauncherPaths,
    session: &MinecraftSession,
    progress: F,
) -> Result<(), BootstrapError>
where
    F: FnMut(BootstrapProgress),
{
    let mut progress = ProgressReporter::new(progress);
    progress.emit("checking", Some("Minecraft 메타데이터".into()), 0, 0);

    tokio::fs::create_dir_all(&paths.instance_dir).await?;
    tokio::fs::create_dir_all(paths.instance_dir.join("versions").join(MINECRAFT_VERSION)).await?;
    tokio::fs::create_dir_all(paths.instance_dir.join("libraries")).await?;
    tokio::fs::create_dir_all(paths.instance_dir.join("assets/indexes")).await?;
    tokio::fs::create_dir_all(paths.instance_dir.join("assets/objects")).await?;
    tokio::fs::create_dir_all(paths.instance_dir.join("natives")).await?;

    let manifest: VersionManifest = reqwest::get(VERSION_MANIFEST_URL)
        .await?
        .error_for_status()?
        .json()
        .await?;
    let version_url = manifest
        .versions
        .iter()
        .find(|entry| entry.id == MINECRAFT_VERSION)
        .map(|entry| entry.url.clone())
        .ok_or(BootstrapError::MissingVersion)?;
    let details: VersionDetails = reqwest::get(&version_url)
        .await?
        .error_for_status()?
        .json()
        .await?;

    let asset_index_bytes = download_bytes(&details.asset_index.url).await?;
    verify_sha1_optional(
        &asset_index_bytes,
        details.asset_index.sha1.as_deref(),
        "asset index",
    )?;
    let asset_index: AssetIndex = serde_json::from_slice(&asset_index_bytes)?;

    let library_downloads = details
        .libraries
        .iter()
        .filter(|library| rules_allow(&library.rules))
        .filter_map(|library| library.downloads.artifact.as_ref())
        .count();
    let native_downloads = details
        .libraries
        .iter()
        .filter(|library| rules_allow(&library.rules))
        .filter_map(|library| {
            native_key(library).and_then(|key| library.downloads.classifiers.get(&key))
        })
        .count();
    progress.set_total(3 + library_downloads + native_downloads + asset_index.objects.len());

    let java_executable = ensure_java_runtime(paths, &mut progress).await?;

    let mut classpath = Vec::new();
    for library in &details.libraries {
        if !rules_allow(&library.rules) {
            continue;
        }

        if let Some(artifact) = &library.downloads.artifact {
            let relative = artifact
                .path
                .clone()
                .unwrap_or_else(|| maven_library_path(&library.name));
            let target = paths.instance_dir.join("libraries").join(&relative);
            download_if_needed(artifact, &target, &relative, &mut progress).await?;
            classpath.push(format!("libraries/{}", relative.replace('\\', "/")));
        }

        if let Some(native_key) = native_key(library) {
            if let Some(native) = library.downloads.classifiers.get(&native_key) {
                let relative = native
                    .path
                    .clone()
                    .unwrap_or_else(|| format!("natives/{native_key}.jar"));
                let target = paths.instance_dir.join("libraries").join(&relative);
                download_if_needed(native, &target, &relative, &mut progress).await?;
                extract_natives(&target, &paths.instance_dir.join("natives"))?;
            }
        }
    }

    let client_path = paths
        .instance_dir
        .join("versions")
        .join(MINECRAFT_VERSION)
        .join(format!("{MINECRAFT_VERSION}.jar"));
    download_if_needed(
        &details.downloads.client,
        &client_path,
        &format!("Minecraft {MINECRAFT_VERSION} client"),
        &mut progress,
    )
    .await?;
    classpath.push(format!(
        "versions/{MINECRAFT_VERSION}/{MINECRAFT_VERSION}.jar"
    ));

    let asset_index_path = paths
        .instance_dir
        .join("assets/indexes")
        .join(format!("{}.json", details.asset_index.id));
    if let Some(parent) = asset_index_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(&asset_index_path, &asset_index_bytes).await?;
    progress.installed("asset index");

    download_assets(&asset_index, &paths.instance_dir, &mut progress).await?;

    let config = MinecraftLaunchConfig {
        java_executable: Some(java_executable),
        working_directory: Some(".".into()),
        jvm_args: vec![
            "-Xmx4G".into(),
            "-Dfile.encoding=UTF-8".into(),
            "-Djava.library.path=natives".into(),
            "-Dorg.lwjgl.librarypath=natives".into(),
            "-Dminecraft.launcher.brand=TownRise".into(),
            "-Dminecraft.launcher.version=0.1.5".into(),
        ],
        classpath,
        main_class: details.main_class.clone(),
        game_args: game_args(&details, session),
    };

    let raw = serde_json::to_string_pretty(&config)?;
    tokio::fs::write(paths.launch_config_path(), raw).await?;
    progress.finished();
    Ok(())
}

async fn ensure_java_runtime<F>(
    paths: &LauncherPaths,
    progress: &mut ProgressReporter<F>,
) -> Result<String, BootstrapError>
where
    F: FnMut(BootstrapProgress),
{
    let java_relative = java_executable_relative_path()?;
    let java_path = paths.instance_dir.join(&java_relative);
    if java_path.exists() {
        progress.installed("Java 21 런타임");
        return Ok(java_relative);
    }

    let runtime_dir = paths.instance_dir.join(JAVA_RUNTIME_DIR);
    let tmp_zip = paths.cache_dir.join("java-21-runtime.zip");
    progress.downloading("Java 21 런타임");
    let bytes = download_bytes(java_runtime_url()?).await?;
    tokio::fs::create_dir_all(&paths.cache_dir).await?;
    tokio::fs::write(&tmp_zip, bytes).await?;
    if runtime_dir.exists() {
        tokio::fs::remove_dir_all(&runtime_dir).await?;
    }
    tokio::fs::create_dir_all(&runtime_dir).await?;
    extract_java_runtime_zip(&tmp_zip, &runtime_dir)?;
    if !java_path.exists() {
        return Err(BootstrapError::MissingJavaExecutable);
    }
    progress.installed("Java 21 런타임");
    Ok(java_relative)
}

fn java_runtime_url() -> Result<&'static str, BootstrapError> {
    if cfg!(target_os = "windows") {
        Ok("https://api.adoptium.net/v3/binary/latest/21/ga/windows/x64/jre/hotspot/normal/eclipse?project=jdk")
    } else {
        Err(BootstrapError::UnsupportedJavaRuntime)
    }
}

fn java_executable_relative_path() -> Result<String, BootstrapError> {
    if cfg!(target_os = "windows") {
        Ok(format!("{JAVA_RUNTIME_DIR}/bin/java.exe"))
    } else {
        Err(BootstrapError::UnsupportedJavaRuntime)
    }
}

fn extract_java_runtime_zip(zip_path: &Path, runtime_dir: &Path) -> Result<(), BootstrapError> {
    let file = std::fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    let root_prefix = common_zip_root(&mut archive);
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let name = entry.name().replace('\\', "/");
        if name.ends_with('/') {
            continue;
        }
        let relative = if let Some(prefix) = &root_prefix {
            name.strip_prefix(prefix).unwrap_or(&name)
        } else {
            &name
        };
        if relative.is_empty() || relative.contains("..") {
            continue;
        }
        let target = runtime_dir.join(relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        std::fs::write(target, bytes)?;
    }
    Ok(())
}

fn common_zip_root(archive: &mut zip::ZipArchive<std::fs::File>) -> Option<String> {
    let mut root: Option<String> = None;
    for index in 0..archive.len() {
        let Ok(entry) = archive.by_index(index) else {
            continue;
        };
        let name = entry.name().replace('\\', "/");
        let Some(first) = name.split('/').next() else {
            continue;
        };
        if first.is_empty() {
            continue;
        }
        let candidate = format!("{first}/");
        match &root {
            None => root = Some(candidate),
            Some(existing) if existing == &candidate => {}
            Some(_) => return None,
        }
    }
    root
}

fn game_args(details: &VersionDetails, session: &MinecraftSession) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(arguments) = &details.arguments {
        for argument in &arguments.game {
            match argument {
                ArgumentValue::Plain(value) => args.push(substitute_arg(value, details, session)),
                ArgumentValue::Conditional { rules, value } if rules_allow(rules) => match value {
                    ArgumentPayload::One(value) => {
                        args.push(substitute_arg(value, details, session))
                    }
                    ArgumentPayload::Many(values) => {
                        args.extend(
                            values
                                .iter()
                                .map(|value| substitute_arg(value, details, session)),
                        );
                    }
                },
                _ => {}
            }
        }
    } else if let Some(legacy) = &details.minecraft_arguments {
        args.extend(
            legacy
                .split_whitespace()
                .map(|value| substitute_arg(value, details, session)),
        );
    }
    args
}

fn substitute_arg(raw: &str, details: &VersionDetails, session: &MinecraftSession) -> String {
    raw.replace("${auth_player_name}", &session.username)
        .replace("${version_name}", &format!("TownRise-{}", details.id))
        .replace("${game_directory}", ".")
        .replace("${assets_root}", "assets")
        .replace("${assets_index_name}", &details.asset_index.id)
        .replace("${auth_uuid}", &session.uuid)
        .replace("${auth_access_token}", &session.access_token)
        .replace("${clientid}", &session.xuid)
        .replace("${auth_xuid}", &session.xuid)
        .replace("${user_type}", "msa")
        .replace("${version_type}", "release")
}

async fn download_assets<F>(
    asset_index: &AssetIndex,
    instance_dir: &Path,
    progress: &mut ProgressReporter<F>,
) -> Result<(), BootstrapError>
where
    F: FnMut(BootstrapProgress),
{
    for object in asset_index.objects.values() {
        let prefix = &object.hash[..2];
        let target = instance_dir
            .join("assets/objects")
            .join(prefix)
            .join(&object.hash);
        let url = format!(
            "https://resources.download.minecraft.net/{}/{}",
            prefix, object.hash
        );
        let download = DownloadInfo {
            path: None,
            url,
            sha1: Some(object.hash.clone()),
        };
        download_if_needed(
            &download,
            &target,
            &format!("asset/{}", object.hash),
            progress,
        )
        .await?;
    }
    Ok(())
}

async fn download_if_needed<F>(
    download: &DownloadInfo,
    target: &Path,
    label: &str,
    progress: &mut ProgressReporter<F>,
) -> Result<(), BootstrapError>
where
    F: FnMut(BootstrapProgress),
{
    if target.exists() {
        if let Some(expected) = &download.sha1 {
            let bytes = tokio::fs::read(target).await?;
            if sha1_hex(&bytes).eq_ignore_ascii_case(expected) {
                progress.installed(label.to_string());
                return Ok(());
            }
        } else {
            progress.installed(label.to_string());
            return Ok(());
        }
    }

    progress.downloading(label.to_string());
    let bytes = download_bytes(&download.url).await?;
    verify_sha1_optional(&bytes, download.sha1.as_deref(), &download.url)?;
    if let Some(parent) = target.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(target, bytes).await?;
    progress.installed(label.to_string());
    Ok(())
}

async fn download_bytes(url: &str) -> Result<Vec<u8>, BootstrapError> {
    Ok(reqwest::get(url)
        .await?
        .error_for_status()?
        .bytes()
        .await?
        .to_vec())
}

fn verify_sha1_optional(
    bytes: &[u8],
    expected: Option<&str>,
    path: &str,
) -> Result<(), BootstrapError> {
    if let Some(expected) = expected {
        if !sha1_hex(bytes).eq_ignore_ascii_case(expected) {
            return Err(BootstrapError::HashMismatch { path: path.into() });
        }
    }
    Ok(())
}

fn sha1_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn extract_natives(native_jar: &Path, natives_dir: &Path) -> Result<(), BootstrapError> {
    std::fs::create_dir_all(natives_dir)?;
    let file = std::fs::File::open(native_jar)?;
    let mut archive = zip::ZipArchive::new(file)?;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let name = entry.name().replace('\\', "/");
        if name.starts_with("META-INF/") || name.ends_with('/') {
            continue;
        }
        let Some(file_name) = Path::new(&name).file_name() else {
            continue;
        };
        let target = natives_dir.join(file_name);
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        std::fs::write(target, bytes)?;
    }
    Ok(())
}

fn native_key(library: &Library) -> Option<String> {
    let key = library.natives.get(current_os_key())?;
    Some(key.replace("${arch}", current_arch()))
}

fn current_os_key() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "osx"
    } else {
        "linux"
    }
}

fn current_arch() -> &'static str {
    if cfg!(target_pointer_width = "64") {
        "64"
    } else {
        "32"
    }
}

fn rules_allow(rules: &[Rule]) -> bool {
    if rules.is_empty() {
        return true;
    }

    let mut allowed = false;
    for rule in rules {
        if rule_applies(rule) {
            allowed = rule.action == "allow";
        }
    }
    allowed
}

fn rule_applies(rule: &Rule) -> bool {
    match &rule.os {
        None => true,
        Some(os) => os
            .name
            .as_deref()
            .is_none_or(|name| name == current_os_key()),
    }
}

fn maven_library_path(name: &str) -> String {
    let parts: Vec<&str> = name.split(':').collect();
    if parts.len() < 3 {
        return name.replace(':', "/");
    }
    let group = parts[0].replace('.', "/");
    let artifact = parts[1];
    let version = parts[2];
    format!("{group}/{artifact}/{version}/{artifact}-{version}.jar")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitutes_offline_arguments() {
        let details = VersionDetails {
            id: "1.21.1".into(),
            main_class: "net.minecraft.client.main.Main".into(),
            asset_index: AssetIndexInfo {
                id: "17".into(),
                url: "https://example.com".into(),
                sha1: None,
            },
            downloads: ClientDownloads {
                client: DownloadInfo {
                    path: None,
                    url: "https://example.com/client.jar".into(),
                    sha1: None,
                },
            },
            libraries: vec![],
            arguments: Some(Arguments {
                game: vec![
                    ArgumentValue::Plain("--username".into()),
                    ArgumentValue::Plain("${auth_player_name}".into()),
                    ArgumentValue::Plain("--assetIndex".into()),
                    ArgumentValue::Plain("${assets_index_name}".into()),
                ],
            }),
            minecraft_arguments: None,
        };

        let session = MinecraftSession {
            username: "TownRisePlayer".into(),
            uuid: "00000000000000000000000000000000".into(),
            access_token: "token".into(),
            xuid: "xuid".into(),
            expires_at: 9999999999,
        };

        assert_eq!(
            game_args(&details, &session),
            vec!["--username", "TownRisePlayer", "--assetIndex", "17"]
        );
    }

    #[test]
    fn creates_maven_library_path() {
        assert_eq!(
            maven_library_path("org.lwjgl:lwjgl:3.3.3"),
            "org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3.jar"
        );
    }
}
