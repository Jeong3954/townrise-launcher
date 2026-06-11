use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use thiserror::Error;
use url::Url;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Manifest {
    pub version: String,
    pub minecraft: String,
    pub loader: LoaderInfo,
    pub files: Vec<ManifestFile>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct LoaderInfo {
    #[serde(rename = "type")]
    pub loader_type: String,
    pub version: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ManifestFile {
    pub path: String,
    pub name: Option<String>,
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ManifestValidationError {
    #[error("unsafe file path in manifest")]
    UnsafePath,
    #[error("invalid file URL in manifest")]
    InvalidUrl,
    #[error("invalid SHA-256 in manifest")]
    InvalidSha256,
}

impl ManifestFile {
    pub fn safe_relative_path(&self) -> Result<PathBuf, ManifestValidationError> {
        safe_relative_path(&self.path)
    }

    pub fn validate(&self) -> Result<(), ManifestValidationError> {
        self.safe_relative_path()?;
        let parsed = Url::parse(&self.url).map_err(|_| ManifestValidationError::InvalidUrl)?;
        match parsed.scheme() {
            "https" => {}
            _ => return Err(ManifestValidationError::InvalidUrl),
        }
        if self.sha256.len() != 64 || !self.sha256.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(ManifestValidationError::InvalidSha256);
        }
        Ok(())
    }
}

pub fn validate_manifest(manifest: &Manifest) -> Result<(), ManifestValidationError> {
    for file in &manifest.files {
        file.validate()?;
    }
    Ok(())
}

pub fn safe_relative_path(raw: &str) -> Result<PathBuf, ManifestValidationError> {
    if raw.trim().is_empty() || raw.contains('\0') {
        return Err(ManifestValidationError::UnsafePath);
    }
    if raw.contains(':') {
        return Err(ManifestValidationError::UnsafePath);
    }

    let path = Path::new(raw);
    if path.is_absolute() {
        return Err(ManifestValidationError::UnsafePath);
    }

    let mut cleaned = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => cleaned.push(part),
            _ => return Err(ManifestValidationError::UnsafePath),
        }
    }

    if cleaned.as_os_str().is_empty() {
        return Err(ManifestValidationError::UnsafePath);
    }

    Ok(cleaned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_manifest_shape() {
        let raw = r#"{
            "version":"0.1.3-dev",
            "minecraft":"1.21.1",
            "loader":{"type":"neoforge","version":"21.1.233"},
            "files":[{"path":"mods/townrise.jar","name":"townrise.jar","url":"https://example.com/townrise.jar","sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","size":10}]
        }"#;
        let manifest: Manifest = serde_json::from_str(raw).expect("manifest should parse");
        assert_eq!(manifest.loader.loader_type, "neoforge");
        assert_eq!(
            manifest.files[0].safe_relative_path().unwrap(),
            PathBuf::from("mods/townrise.jar")
        );
    }

    #[test]
    fn rejects_unsafe_paths() {
        for raw in [
            "../evil.jar",
            "/tmp/evil.jar",
            "C:\\evil.jar",
            "mods/../evil.jar",
            "",
        ] {
            assert!(safe_relative_path(raw).is_err(), "{raw} should be rejected");
        }
    }

    #[test]
    fn rejects_non_https_url() {
        let file = ManifestFile {
            path: "mods/a.jar".into(),
            name: None,
            url: "http://example.com/a.jar".into(),
            sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            size: 1,
        };
        assert_eq!(file.validate(), Err(ManifestValidationError::InvalidUrl));
    }
}
