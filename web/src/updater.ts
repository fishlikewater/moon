/**
 * 桌面端应用内更新：检查、下载安装与重启。
 *
 * Tauri 插件只在函数内动态导入，浏览器开发/测试环境不会加载原生模块。
 * 所有面向用户的错因都经过 `updateErrorMessage` 清洗，避免把栈信息直接展示。
 */

import { isDesktopShell } from "./desktopShell";

/** 与后端无关的更新状态机；UI 只消费这些阶段。 */
export type UpdatePhase =
  | "idle"
  | "checking"
  | "up_to_date"
  | "available"
  | "downloading"
  | "ready"
  | "failed";

export type UpdateInfo = {
  version: string;
  date: string | null;
  body: string | null;
};

export type UpdateProgress = {
  phase: "started" | "progress" | "finished";
  /** 0–100；服务端未给 contentLength 时为 null（不确定进度）。 */
  percent: number | null;
};

/** 一次可用更新的句柄：info 给 UI，下载方法保留插件返回的 Update 对象。 */
export type PendingUpdate = {
  info: UpdateInfo;
  downloadAndInstall: (
    onProgress: (progress: UpdateProgress) => void,
  ) => Promise<void>;
};

/** 版本比较：只在数字段可比时判断，预发布等非数字版本一律视为不更新。 */
export function isNewerVersion(candidate: string, current: string): boolean {
  const parse = (value: string): number[] =>
    value
      .trim()
      .replace(/^v/i, "")
      .split(/[-+]/, 1)[0]
      .split(".")
      .map((part) => Number.parseInt(part, 10));
  const candidateParts = parse(candidate);
  const currentParts = parse(current);
  const length = Math.max(candidateParts.length, currentParts.length);
  for (let index = 0; index < length; index += 1) {
    const left = candidateParts[index] ?? 0;
    const right = currentParts[index] ?? 0;
    if (Number.isNaN(left) || Number.isNaN(right)) {
      return false;
    }
    if (left !== right) {
      return left > right;
    }
  }
  return false;
}

export function updateStatusText(
  phase: UpdatePhase,
  version?: string,
): string {
  switch (phase) {
    case "idle":
      return "尚未检查更新";
    case "checking":
      return "正在检查更新…";
    case "up_to_date":
      return "已是最新版本";
    case "available":
      return version ? `发现新版本 v${version}` : "发现新版本";
    case "downloading":
      return "正在下载更新…";
    case "ready":
      return "更新已下载，正在重启…";
    case "failed":
      return "检查或安装更新失败";
  }
}

/** 进度百分比：未知总量返回 null，其余夹在 0–100 并取整。 */
export function progressPercent(
  received: number,
  total: number | null,
): number | null {
  if (total === null || total <= 0 || !Number.isFinite(received)) {
    return null;
  }
  return Math.max(0, Math.min(100, Math.round((received / total) * 100)));
}

/** 更新说明按纯文本展示：统一换行、去控制字符、限长。 */
export function sanitizeUpdateNotes(body: string | null | undefined): string {
  if (!body) {
    return "";
  }
  return body
    .replace(/\r\n?/g, "\n")
    .replace(/[\u0000-\u0008\u000B\u000C\u000E-\u001F]/g, "")
    .trim()
    .slice(0, 4000);
}

/** 错误消息只保留首行并限长；空消息给出可操作提示。 */
export function updateErrorMessage(cause: unknown): string {
  const raw =
    cause instanceof Error
      ? cause.message
      : typeof cause === "string"
        ? cause
        : "";
  const firstLine = raw.split("\n")[0].trim();
  return firstLine === "" ? "更新失败，请稍后重试" : firstLine.slice(0, 200);
}

/**
 * 检查更新；无更新或非桌面壳返回 null。
 *
 * 插件返回的 Update 对象不进入 React 状态（不可序列化），用句柄包一层。
 */
export async function checkForUpdate(): Promise<PendingUpdate | null> {
  if (!isDesktopShell()) {
    return null;
  }
  const { check } = await import("@tauri-apps/plugin-updater");
  const update = await check();
  if (update === null) {
    return null;
  }
  // 插件已比较版本；这里用本地版本再兜底，避免 latest.json 回退到同版本或旧版本。
  const currentVersion = await getCurrentVersion();
  if (
    currentVersion !== null &&
    !isNewerVersion(update.version, currentVersion)
  ) {
    return null;
  }
  return {
    info: {
      version: update.version,
      date: update.date ?? null,
      body: sanitizeUpdateNotes(update.body ?? null) || null,
    },
    async downloadAndInstall(onProgress) {
      let received = 0;
      let total: number | null = null;
      await update.downloadAndInstall((event) => {
        switch (event.event) {
          case "Started":
            received = 0;
            total = event.data.contentLength ?? null;
            onProgress({
              phase: "started",
              percent: progressPercent(received, total),
            });
            break;
          case "Progress":
            received += event.data.chunkLength;
            onProgress({
              phase: "progress",
              percent: progressPercent(received, total),
            });
            break;
          case "Finished":
            onProgress({ phase: "finished", percent: 100 });
            break;
        }
      });
    },
  };
}

/** 当前应用版本；浏览器模式没有原生版本号。 */
export async function getCurrentVersion(): Promise<string | null> {
  if (!isDesktopShell()) {
    return null;
  }
  try {
    const { getVersion } = await import("@tauri-apps/api/app");
    return await getVersion();
  } catch {
    return null;
  }
}

/** 安装完成后重启应用，使新版本生效。 */
export async function relaunchApp(): Promise<void> {
  const { relaunch } = await import("@tauri-apps/plugin-process");
  await relaunch();
}

export type CheckOutcome =
  | { status: "up_to_date" }
  | { status: "available"; update: PendingUpdate }
  | { status: "failed"; error: string };

/** 检查并把异常映射成 UI 可消费的结果，避免组件里重复 try/catch。 */
export async function runUpdateCheck(): Promise<CheckOutcome> {
  try {
    const update = await checkForUpdate();
    return update === null
      ? { status: "up_to_date" }
      : { status: "available", update };
  } catch (cause) {
    return { status: "failed", error: updateErrorMessage(cause) };
  }
}

export type InstallOutcome = { ok: true } | { ok: false; error: string };

/** 下载安装并在成功后重启；失败时返回清洗后的错误消息。 */
export async function runUpdateInstall(
  update: PendingUpdate,
  onProgress: (progress: UpdateProgress) => void,
): Promise<InstallOutcome> {
  try {
    await update.downloadAndInstall(onProgress);
    await relaunchApp();
    return { ok: true };
  } catch (cause) {
    return { ok: false, error: updateErrorMessage(cause) };
  }
}
