import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { openUrl } from "@tauri-apps/plugin-opener";
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type DownloadEvent } from "@tauri-apps/plugin-updater";

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

export type LauncherSelfUpdateProgress = {
  phase: "checking" | "downloading" | "installing" | "restarting" | "current";
  version?: string;
  downloadedBytes?: number;
  totalBytes?: number;
};

export type MinecraftSession = {
  username: string;
  uuid: string;
  accessToken: string;
  xuid: string;
  expiresAt: number;
};

export type LoginStart = {
  deviceCode: string;
  userCode: string;
  verificationUri: string;
  verificationUriComplete: string | null;
  expiresIn: number;
  interval: number;
  message: string;
};

export type LoginPollStatus =
  | "Pending"
  | "SlowDown"
  | { Complete: MinecraftSession };

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

export async function installLauncherSelfUpdate(
  onProgress: (progress: LauncherSelfUpdateProgress) => void = () => undefined,
): Promise<boolean> {
  if (!isTauri()) return false;

  onProgress({ phase: "checking" });
  const update = await check({ timeout: 60_000 });
  if (!update) {
    onProgress({ phase: "current" });
    return false;
  }

  let downloadedBytes = 0;
  let totalBytes: number | undefined;
  const progressHandler = (event: DownloadEvent) => {
    if (event.event === "Started") {
      downloadedBytes = 0;
      totalBytes = event.data.contentLength;
      onProgress({
        phase: "downloading",
        version: update.version,
        downloadedBytes,
        totalBytes,
      });
    } else if (event.event === "Progress") {
      downloadedBytes += event.data.chunkLength;
      onProgress({
        phase: "downloading",
        version: update.version,
        downloadedBytes,
        totalBytes,
      });
    } else if (event.event === "Finished") {
      onProgress({
        phase: "installing",
        version: update.version,
        downloadedBytes,
        totalBytes,
      });
    }
  };

  try {
    onProgress({ phase: "downloading", version: update.version });
    await update.download(progressHandler, { timeout: 5 * 60_000 });
    onProgress({
      phase: "installing",
      version: update.version,
      downloadedBytes,
      totalBytes,
    });
    await update.install();
  } finally {
    await update.close();
  }

  onProgress({
    phase: "restarting",
    version: update.version,
    downloadedBytes,
    totalBytes,
  });
  await relaunch();
  return true;
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

export async function onMinecraftBootstrapProgress(
  callback: (progress: UpdateProgress) => void,
): Promise<UnlistenFn> {
  if (!isTauri()) return () => undefined;
  return listen<UpdateProgress>("minecraft-bootstrap-progress", (event) =>
    callback(event.payload),
  );
}

export async function currentMinecraftSession(): Promise<MinecraftSession | null> {
  if (!isTauri()) return null;
  return invoke<MinecraftSession | null>("current_minecraft_session");
}

export async function beginMicrosoftLogin(): Promise<LoginStart> {
  if (!isTauri()) {
    return {
      deviceCode: "dev-preview",
      userCode: "",
      verificationUri: "about:blank",
      verificationUriComplete: "about:blank",
      expiresIn: 900,
      interval: 5,
      message:
        "Tauri 앱에서는 공식 Microsoft 로그인 화면이 브라우저로 열립니다.",
    };
  }
  return invoke<LoginStart>("begin_microsoft_login");
}

export async function pollMicrosoftLogin(
  deviceCode: string,
): Promise<LoginPollStatus> {
  if (!isTauri()) return "Pending";
  return invoke<LoginPollStatus>("poll_microsoft_login", { deviceCode });
}

export async function logoutMinecraft(): Promise<void> {
  if (!isTauri()) return;
  await invoke("logout_minecraft");
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

export async function openExternalUrl(url: string): Promise<void> {
  if (!isTauri()) {
    window.open(url, "_blank", "noopener,noreferrer");
    return;
  }
  await openUrl(url);
}
