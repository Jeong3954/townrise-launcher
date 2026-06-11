pub mod launcher;
pub mod manifest;
pub mod updater;

use launcher::{launch_minecraft, LauncherPaths};
use tauri::Emitter;
use updater::{install_updates_to_with_progress, plan_updates};

const DEFAULT_MANIFEST_URL: &str =
    "https://github.com/Jeong3954/townrise-mod/releases/latest/download/manifest.json";

#[tauri::command]
async fn check_for_updates() -> Result<updater::UpdatePlan, String> {
    let paths = LauncherPaths::discover().map_err(|error| error.to_string())?;
    plan_updates(DEFAULT_MANIFEST_URL, &paths.instance_dir)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn install_updates(app: tauri::AppHandle) -> Result<updater::InstallSummary, String> {
    let paths = LauncherPaths::discover().map_err(|error| error.to_string())?;
    install_updates_to_with_progress(
        DEFAULT_MANIFEST_URL,
        &paths.instance_dir,
        &paths.cache_dir,
        |progress| {
            let _ = app.emit("update-progress", progress);
        },
    )
    .await
    .map_err(|error| error.to_string())
}

#[tauri::command]
async fn launch_game() -> Result<u32, String> {
    let paths = LauncherPaths::discover().map_err(|error| error.to_string())?;
    launch_minecraft(&paths).map_err(|error| error.to_string())
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            check_for_updates,
            install_updates,
            launch_game
        ])
        .run(tauri::generate_context!())
        .expect("failed to run TownRise launcher");
}
