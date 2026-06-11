use crate::manifest::{validate_manifest, Manifest, ManifestFile, ManifestValidationError};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use thiserror::Error;
use tokio::io::AsyncWriteExt;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpdatePlan {
    pub version: String,
    pub update_required: bool,
    pub files: Vec<UpdateFile>,
    pub total_download_size: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateFile {
    pub path: String,
    pub status: UpdateFileStatus,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum UpdateFileStatus {
    Current,
    Missing,
    Outdated,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InstallSummary {
    pub version: String,
    pub installed: usize,
    pub skipped: usize,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProgress {
    pub phase: String,
    pub current_file: Option<String>,
    pub completed_files: usize,
    pub total_files: usize,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub percent: u8,
}

#[derive(Debug, Error)]
pub enum UpdateError {
    #[error("manifest request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("manifest validation failed: {0}")]
    Manifest(#[from] ManifestValidationError),
    #[error("file operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("downloaded file size mismatch for {path}")]
    SizeMismatch { path: String },
    #[error("downloaded file hash mismatch for {path}")]
    HashMismatch { path: String },
}

pub async fn fetch_manifest(url: &str) -> Result<Manifest, UpdateError> {
    let manifest = reqwest::get(url)
        .await?
        .error_for_status()?
        .json::<Manifest>()
        .await?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

pub async fn plan_updates(
    manifest_url: &str,
    instance_dir: &Path,
) -> Result<UpdatePlan, UpdateError> {
    let manifest = fetch_manifest(manifest_url).await?;
    plan_manifest_updates(&manifest, instance_dir).await
}

pub async fn plan_manifest_updates(
    manifest: &Manifest,
    instance_dir: &Path,
) -> Result<UpdatePlan, UpdateError> {
    validate_manifest(manifest)?;
    let mut files = Vec::with_capacity(manifest.files.len());
    let mut total_download_size = 0;

    for file in &manifest.files {
        let relative = file.safe_relative_path()?;
        let target = instance_dir.join(&relative);
        let status = classify_file(&target, file).await?;
        if status != UpdateFileStatus::Current {
            total_download_size += file.size;
        }
        files.push(UpdateFile {
            path: file.path.clone(),
            status,
            size: file.size,
        });
    }

    Ok(UpdatePlan {
        version: manifest.version.clone(),
        update_required: files
            .iter()
            .any(|file| file.status != UpdateFileStatus::Current),
        files,
        total_download_size,
    })
}

pub async fn install_updates_to(
    manifest_url: &str,
    instance_dir: &Path,
    cache_dir: &Path,
) -> Result<InstallSummary, UpdateError> {
    let manifest = fetch_manifest(manifest_url).await?;
    install_manifest_updates(&manifest, instance_dir, cache_dir).await
}

pub async fn install_updates_to_with_progress<F>(
    manifest_url: &str,
    instance_dir: &Path,
    cache_dir: &Path,
    on_progress: F,
) -> Result<InstallSummary, UpdateError>
where
    F: FnMut(UpdateProgress),
{
    let manifest = fetch_manifest(manifest_url).await?;
    install_manifest_updates_with_progress(&manifest, instance_dir, cache_dir, on_progress).await
}

pub async fn install_manifest_updates(
    manifest: &Manifest,
    instance_dir: &Path,
    cache_dir: &Path,
) -> Result<InstallSummary, UpdateError> {
    install_manifest_updates_with_progress(manifest, instance_dir, cache_dir, |_| {}).await
}

pub async fn install_manifest_updates_with_progress<F>(
    manifest: &Manifest,
    instance_dir: &Path,
    cache_dir: &Path,
    mut on_progress: F,
) -> Result<InstallSummary, UpdateError>
where
    F: FnMut(UpdateProgress),
{
    validate_manifest(manifest)?;
    tokio::fs::create_dir_all(instance_dir).await?;
    tokio::fs::create_dir_all(cache_dir).await?;

    let mut installed = 0;
    let mut skipped = 0;
    let mut downloaded_bytes = 0;
    let mut installed_bytes = 0;
    let mut completed_files = 0;
    let total_bytes = manifest.files.iter().map(|file| file.size).sum();
    let total_files = manifest.files.len();

    emit_progress(
        &mut on_progress,
        "starting",
        None,
        completed_files,
        total_files,
        downloaded_bytes,
        total_bytes,
    );

    for file in &manifest.files {
        let relative = file.safe_relative_path()?;
        let target = instance_dir.join(&relative);
        emit_progress(
            &mut on_progress,
            "checking",
            Some(file.path.clone()),
            completed_files,
            total_files,
            downloaded_bytes,
            total_bytes,
        );
        if classify_file(&target, file).await? == UpdateFileStatus::Current {
            skipped += 1;
            completed_files += 1;
            downloaded_bytes += file.size;
            emit_progress(
                &mut on_progress,
                "skipped",
                Some(file.path.clone()),
                completed_files,
                total_files,
                downloaded_bytes,
                total_bytes,
            );
            continue;
        }

        emit_progress(
            &mut on_progress,
            "downloading",
            Some(file.path.clone()),
            completed_files,
            total_files,
            downloaded_bytes,
            total_bytes,
        );
        let tmp = cache_dir.join(temp_name(file));
        download_file(file, &tmp).await?;
        verify_download(file, &tmp).await?;

        if let Some(parent) = target.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::rename(&tmp, &target).await?;
        installed += 1;
        completed_files += 1;
        downloaded_bytes += file.size;
        installed_bytes += file.size;
        emit_progress(
            &mut on_progress,
            "installed",
            Some(file.path.clone()),
            completed_files,
            total_files,
            downloaded_bytes,
            total_bytes,
        );
    }

    emit_progress(
        &mut on_progress,
        "finished",
        None,
        completed_files,
        total_files,
        downloaded_bytes,
        total_bytes,
    );

    Ok(InstallSummary {
        version: manifest.version.clone(),
        installed,
        skipped,
        total_bytes: installed_bytes,
    })
}

async fn classify_file(
    target: &Path,
    file: &ManifestFile,
) -> Result<UpdateFileStatus, UpdateError> {
    if !target.exists() {
        return Ok(UpdateFileStatus::Missing);
    }
    let metadata = tokio::fs::metadata(target).await?;
    if metadata.len() != file.size {
        return Ok(UpdateFileStatus::Outdated);
    }
    let sha = sha256_file(target).await?;
    if sha.eq_ignore_ascii_case(&file.sha256) {
        Ok(UpdateFileStatus::Current)
    } else {
        Ok(UpdateFileStatus::Outdated)
    }
}

async fn download_file(file: &ManifestFile, tmp: &Path) -> Result<(), UpdateError> {
    if let Some(parent) = tmp.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let bytes = reqwest::get(&file.url)
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    let mut output = tokio::fs::File::create(tmp).await?;
    output.write_all(&bytes).await?;
    output.flush().await?;
    Ok(())
}

async fn verify_download(file: &ManifestFile, tmp: &Path) -> Result<(), UpdateError> {
    let metadata = tokio::fs::metadata(tmp).await?;
    if metadata.len() != file.size {
        return Err(UpdateError::SizeMismatch {
            path: file.path.clone(),
        });
    }
    let actual = sha256_file(tmp).await?;
    if !actual.eq_ignore_ascii_case(&file.sha256) {
        return Err(UpdateError::HashMismatch {
            path: file.path.clone(),
        });
    }
    Ok(())
}

async fn sha256_file(path: &Path) -> Result<String, UpdateError> {
    let bytes = tokio::fs::read(path).await?;
    let digest = Sha256::digest(bytes);
    Ok(hex::encode(digest))
}

fn temp_name(file: &ManifestFile) -> PathBuf {
    let safe_name = file.path.replace(['/', '\\'], "_");
    PathBuf::from(format!("{safe_name}.download"))
}

fn emit_progress<F>(
    on_progress: &mut F,
    phase: &str,
    current_file: Option<String>,
    completed_files: usize,
    total_files: usize,
    downloaded_bytes: u64,
    total_bytes: u64,
) where
    F: FnMut(UpdateProgress),
{
    let percent = downloaded_bytes
        .saturating_mul(100)
        .checked_div(total_bytes)
        .unwrap_or(100)
        .min(100) as u8;
    on_progress(UpdateProgress {
        phase: phase.to_string(),
        current_file,
        completed_files,
        total_files,
        downloaded_bytes,
        total_bytes,
        percent,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{LoaderInfo, ManifestFile};
    use sha2::{Digest, Sha256};
    use tempfile::tempdir;

    fn test_manifest(bytes: &[u8]) -> Manifest {
        Manifest {
            version: "test".into(),
            minecraft: "1.21.1".into(),
            loader: LoaderInfo {
                loader_type: "neoforge".into(),
                version: "21.1.233".into(),
            },
            files: vec![ManifestFile {
                path: "mods/townrise.jar".into(),
                name: Some("townrise.jar".into()),
                url: "https://example.com/townrise.jar".into(),
                sha256: hex::encode(Sha256::digest(bytes)),
                size: bytes.len() as u64,
            }],
        }
    }

    #[tokio::test]
    async fn plans_missing_file() {
        let dir = tempdir().unwrap();
        let manifest = test_manifest(b"abc");
        let plan = plan_manifest_updates(&manifest, dir.path()).await.unwrap();
        assert!(plan.update_required);
        assert_eq!(plan.files[0].status, UpdateFileStatus::Missing);
    }

    #[tokio::test]
    async fn plans_current_file() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("mods/townrise.jar");
        tokio::fs::create_dir_all(target.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&target, b"abc").await.unwrap();
        let manifest = test_manifest(b"abc");
        let plan = plan_manifest_updates(&manifest, dir.path()).await.unwrap();
        assert!(!plan.update_required);
        assert_eq!(plan.files[0].status, UpdateFileStatus::Current);
    }

    #[tokio::test]
    async fn rejects_hash_mismatch_on_verify() {
        let dir = tempdir().unwrap();
        let tmp = dir.path().join("bad.jar");
        tokio::fs::write(&tmp, b"bad").await.unwrap();
        let manifest = test_manifest(b"abc");
        let err = verify_download(&manifest.files[0], &tmp).await.unwrap_err();
        assert!(matches!(
            err,
            UpdateError::HashMismatch { .. } | UpdateError::SizeMismatch { .. }
        ));
    }
}
