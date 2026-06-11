use crate::launcher::{LauncherPaths, MinecraftLaunchConfig};
use serde::Deserialize;
use sha1::{Digest, Sha1};
use std::{collections::HashMap, io::Read, path::Path};
use thiserror::Error;

const MINECRAFT_VERSION: &str = "1.21.1";
const VERSION_MANIFEST_URL: &str =
    "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

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

pub async fn prepare_default_instance(paths: &LauncherPaths) -> Result<(), BootstrapError> {
    if paths.launch_config_path().exists() {
        return Ok(());
    }

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
            download_if_needed(artifact, &target).await?;
            classpath.push(format!("libraries/{}", relative.replace('\\', "/")));
        }

        if let Some(native_key) = native_key(library) {
            if let Some(native) = library.downloads.classifiers.get(&native_key) {
                let relative = native
                    .path
                    .clone()
                    .unwrap_or_else(|| format!("natives/{native_key}.jar"));
                let target = paths.instance_dir.join("libraries").join(&relative);
                download_if_needed(native, &target).await?;
                extract_natives(&target, &paths.instance_dir.join("natives"))?;
            }
        }
    }

    let client_path = paths
        .instance_dir
        .join("versions")
        .join(MINECRAFT_VERSION)
        .join(format!("{MINECRAFT_VERSION}.jar"));
    download_if_needed(&details.downloads.client, &client_path).await?;
    classpath.push(format!(
        "versions/{MINECRAFT_VERSION}/{MINECRAFT_VERSION}.jar"
    ));

    let asset_index_path = paths
        .instance_dir
        .join("assets/indexes")
        .join(format!("{}.json", details.asset_index.id));
    let asset_index_bytes = download_bytes(&details.asset_index.url).await?;
    verify_sha1_optional(
        &asset_index_bytes,
        details.asset_index.sha1.as_deref(),
        "asset index",
    )?;
    if let Some(parent) = asset_index_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(&asset_index_path, &asset_index_bytes).await?;
    let asset_index: AssetIndex = serde_json::from_slice(&asset_index_bytes)?;
    download_assets(&asset_index, &paths.instance_dir).await?;

    let config = MinecraftLaunchConfig {
        java_executable: Some("java".into()),
        working_directory: Some(".".into()),
        jvm_args: vec![
            "-Xmx4G".into(),
            "-Dfile.encoding=UTF-8".into(),
            "-Djava.library.path=natives".into(),
            "-Dorg.lwjgl.librarypath=natives".into(),
            "-Dminecraft.launcher.brand=TownRise".into(),
            "-Dminecraft.launcher.version=0.1.1".into(),
        ],
        classpath,
        main_class: details.main_class.clone(),
        game_args: game_args(&details),
    };

    let raw = serde_json::to_string_pretty(&config)?;
    tokio::fs::write(paths.launch_config_path(), raw).await?;
    Ok(())
}

fn game_args(details: &VersionDetails) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(arguments) = &details.arguments {
        for argument in &arguments.game {
            match argument {
                ArgumentValue::Plain(value) => args.push(substitute_arg(value, details)),
                ArgumentValue::Conditional { rules, value } if rules_allow(rules) => match value {
                    ArgumentPayload::One(value) => args.push(substitute_arg(value, details)),
                    ArgumentPayload::Many(values) => {
                        args.extend(values.iter().map(|value| substitute_arg(value, details)));
                    }
                },
                _ => {}
            }
        }
    } else if let Some(legacy) = &details.minecraft_arguments {
        args.extend(
            legacy
                .split_whitespace()
                .map(|value| substitute_arg(value, details)),
        );
    }
    args
}

fn substitute_arg(raw: &str, details: &VersionDetails) -> String {
    raw.replace("${auth_player_name}", "TownRisePlayer")
        .replace("${version_name}", &format!("TownRise-{}", details.id))
        .replace("${game_directory}", ".")
        .replace("${assets_root}", "assets")
        .replace("${assets_index_name}", &details.asset_index.id)
        .replace("${auth_uuid}", "00000000-0000-0000-0000-000000000000")
        .replace("${auth_access_token}", "0")
        .replace("${clientid}", "0")
        .replace("${auth_xuid}", "0")
        .replace("${user_type}", "legacy")
        .replace("${version_type}", "release")
}

async fn download_assets(
    asset_index: &AssetIndex,
    instance_dir: &Path,
) -> Result<(), BootstrapError> {
    for object in asset_index.objects.values() {
        let prefix = &object.hash[..2];
        let target = instance_dir
            .join("assets/objects")
            .join(prefix)
            .join(&object.hash);
        if target.exists() {
            continue;
        }
        let url = format!(
            "https://resources.download.minecraft.net/{}/{}",
            prefix, object.hash
        );
        let download = DownloadInfo {
            path: None,
            url,
            sha1: Some(object.hash.clone()),
        };
        download_if_needed(&download, &target).await?;
    }
    Ok(())
}

async fn download_if_needed(download: &DownloadInfo, target: &Path) -> Result<(), BootstrapError> {
    if target.exists() {
        if let Some(expected) = &download.sha1 {
            let bytes = tokio::fs::read(target).await?;
            if sha1_hex(&bytes).eq_ignore_ascii_case(expected) {
                return Ok(());
            }
        } else {
            return Ok(());
        }
    }

    let bytes = download_bytes(&download.url).await?;
    verify_sha1_optional(&bytes, download.sha1.as_deref(), &download.url)?;
    if let Some(parent) = target.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(target, bytes).await?;
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

        assert_eq!(
            game_args(&details),
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
