import { invoke } from "@tauri-apps/api/core";

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

const fallbackPlan: UpdatePlan = {
  version: "확인 전",
  updateRequired: false,
  files: [],
  totalDownloadSize: 0,
};

export async function checkForUpdates(): Promise<UpdatePlan> {
  if (!("__TAURI_INTERNALS__" in window)) return fallbackPlan;
  return invoke<UpdatePlan>("check_for_updates");
}

export async function installUpdates(): Promise<InstallSummary> {
  if (!("__TAURI_INTERNALS__" in window)) {
    return { version: "dev-preview", installed: 0, skipped: 0, totalBytes: 0 };
  }
  return invoke<InstallSummary>("install_updates");
}

export async function launchGame(): Promise<void> {
  if (!("__TAURI_INTERNALS__" in window)) {
    throw new Error("브라우저 미리보기에서는 게임 실행을 사용할 수 없습니다.");
  }
  return invoke<void>("launch_game");
}
