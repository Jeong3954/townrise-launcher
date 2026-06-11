# AGENTS.md

## Project

TownRise Launcher is a dedicated desktop launcher for **TownRise: 시장의 시대**, built with Tauri, SvelteKit, TypeScript, and Rust.

Primary target: Windows 10/11.
Development may run on Linux, but do not optimize UI/packaging around Linux first.

## Product Direction

Make the launcher feel strongly Minecraft-inspired and pixel-art themed, but keep it polished and easy to use.

Use Korean-first copy.

Prefer:

- Pixel-style panels, borders, buttons, and visual rhythm
- Minecraft-like earthy palette: deep stone, dirt brown, grass green, torch gold
- Clear main action: `게임 시작`
- Simple update status: `최신 상태`, `업데이트 필요`, `업데이트 중`, `실패`
- Patch notes and server status as user-facing cards

Avoid showing internal details in the main UI:

- Do not expose raw SHA-256 values, internal file paths, loader internals, or stack traces in normal screens
- Put technical details only in logs or a compact developer/debug area if needed
- Do not make the UI feel like a generic dev tool

## Initial MVP Scope

Implement a working launcher MVP:

1. Main pixel-themed Korean UI.
2. Fetch the TownRise update manifest.
3. Plan missing/outdated files.
4. Download files to a dedicated instance directory.
5. Verify SHA-256 and size before install.
6. Reject unsafe manifest paths.
7. Provide `게임 시작` button, but Minecraft launch can be a clear stub for now.

Do not implement Microsoft login yet.
Do not implement payments, anti-cheat, or whitelist systems yet.

## Default Manifest URL

Use:

```text
https://github.com/Jeong3954/townrise-mod/releases/latest/download/manifest.json
```

## Security Rules

Remote manifest is untrusted.

- Reject absolute paths.
- Reject paths containing `..`.
- Reject Windows drive paths like `C:\...`.
- Only write under the TownRise launcher instance directory.
- Download to temporary files first.
- Verify SHA-256 and size before replacing files.
- Never install a file if verification fails.
- Do not hardcode secrets.

## Local Data Directory

Use a dedicated app data directory. Instance files belong under:

```text
TownRiseLauncher/instance/
```

## Rust Backend

Rust owns filesystem and update safety logic.
Expose Tauri commands:

```rust
check_for_updates() -> Result<UpdatePlan, String>
install_updates() -> Result<InstallSummary, String>
launch_game() -> Result<(), String>
```

Keep update logic testable outside Tauri where possible.

## Frontend

SvelteKit owns presentation only:

- Pixel-themed layout
- Buttons and status cards
- Progress/result messages
- Friendly Korean errors

Do not duplicate critical path validation in TypeScript only.

## Verification

Before finishing a task, run what is available:

```bash
npm run check
npm run build
cargo fmt --check
cargo test
cargo check
```

If Tauri GUI packaging cannot be verified on the current Linux environment due to missing system WebKit/GTK packages, clearly report that and still verify Rust core + frontend build.
