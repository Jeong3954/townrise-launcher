pub mod auth;
pub mod launcher;
pub mod manifest;
pub mod minecraft_bootstrap;
pub mod updater;

use auth::{
    begin_login, load_session, logout, poll_login, LoginPollStatus, LoginStart, MinecraftSession,
};
use launcher::{launch_minecraft, LauncherPaths};
use minecraft_bootstrap::prepare_default_instance_with_progress;
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
async fn current_minecraft_session() -> Result<Option<MinecraftSession>, String> {
    let paths = LauncherPaths::discover().map_err(|error| error.to_string())?;
    load_session(&paths)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn begin_microsoft_login() -> Result<LoginStart, String> {
    let paths = LauncherPaths::discover().map_err(|error| error.to_string())?;
    begin_login(&paths).await.map_err(|error| error.to_string())
}

#[tauri::command]
async fn poll_microsoft_login(device_code: String) -> Result<LoginPollStatus, String> {
    let paths = LauncherPaths::discover().map_err(|error| error.to_string())?;
    poll_login(&paths, &device_code)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn logout_minecraft() -> Result<(), String> {
    let paths = LauncherPaths::discover().map_err(|error| error.to_string())?;
    logout(&paths).await.map_err(|error| error.to_string())
}

#[tauri::command]
async fn launch_game(app: tauri::AppHandle) -> Result<u32, String> {
    let paths = LauncherPaths::discover().map_err(|error| error.to_string())?;
    let session = load_session(&paths)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Microsoft login is required before launching Minecraft".to_string())?;
    prepare_default_instance_with_progress(&paths, &session, |progress| {
        let _ = app.emit("minecraft-bootstrap-progress", progress);
    })
    .await
    .map_err(|error| error.to_string())?;
    launch_minecraft(&paths).map_err(|error| error.to_string())
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            check_for_updates,
            install_updates,
            current_minecraft_session,
            begin_microsoft_login,
            poll_microsoft_login,
            logout_minecraft,
            launch_game
        ])
        .run(tauri::generate_context!())
        .expect("failed to run TownRise launcher");
}
