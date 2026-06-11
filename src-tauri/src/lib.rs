pub mod launcher;
pub mod manifest;
pub mod updater;

use launcher::LauncherPaths;
use updater::{install_updates_to, plan_updates};

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
async fn install_updates() -> Result<updater::InstallSummary, String> {
    let paths = LauncherPaths::discover().map_err(|error| error.to_string())?;
    install_updates_to(DEFAULT_MANIFEST_URL, &paths.instance_dir, &paths.cache_dir)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn launch_game() -> Result<(), String> {
    Err("not implemented: Minecraft launch will be connected after update MVP".to_string())
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
