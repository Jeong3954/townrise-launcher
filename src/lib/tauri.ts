import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

export type UpdateFileStatus = "current" | "missing" | "outdated";

export type UpdateFile = {
  path: string;
  status: UpdateFileStatus;
  size: number;
};

export type UpdatePlan = {
  version: string;
  updateRequired: boolean;
  files: UpdateFile[];
  totalDownloadSize: number;
};

export type InstallSummary = {
  version: string;
  installed: number;
  skipped: number;
  totalBytes: number;
};

export type UpdateProgress = {
  phase: string;
  currentFile: string | null;
  completedFiles: number;
  totalFiles: number;
  downloadedBytes: number;
  totalBytes: number;
  percent: number;
};

const fallbackPlan: UpdatePlan = {
  version: "확인 전",
  updateRequired: false,
  files: [],
  totalDownloadSize: 0,
};

function isTauri() {
  return "__TAURI_INTERNALS__" in window;
}

export async function checkForUpdates(): Promise<UpdatePlan> {
  if (!isTauri()) return fallbackPlan;
  return invoke<UpdatePlan>("check_for_updates");
}

export async function installUpdates(): Promise<InstallSummary> {
  if (!isTauri()) {
    return { version: "dev-preview", installed: 0, skipped: 0, totalBytes: 0 };
  }
  return invoke<InstallSummary>("install_updates");
}

export async function onUpdateProgress(
  callback: (progress: UpdateProgress) => void,
): Promise<UnlistenFn> {
  if (!isTauri()) return () => undefined;
  return listen<UpdateProgress>("update-progress", (event) =>
    callback(event.payload),
  );
}

export async function launchGame(): Promise<number> {
  if (!isTauri()) {
    throw new Error("브라우저 미리보기에서는 게임 실행을 사용할 수 없습니다.");
  }
  return invoke<number>("launch_game");
}

export async function minimizeWindow(): Promise<void> {
  if (!isTauri()) return;
  await getCurrentWindow().minimize();
}

export async function startWindowDrag(): Promise<void> {
  if (!isTauri()) return;
  await getCurrentWindow().startDragging();
}

export async function closeWindow(): Promise<void> {
  if (!isTauri()) return;
  await getCurrentWindow().close();
}
