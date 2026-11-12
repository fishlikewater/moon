import type {
  AppSettings,
  HarnessBinarySource,
  HarnessBinaryStatus,
  RoomPermissionMode,
} from "@moon/shared";

/** 与后端 domain/settings.rs 保持一致的边界值。 */
export const SETTINGS_LIMITS = {
  discussionRounds: { min: 1, max: 50 },
  contextBudgetChars: { min: 2_000, max: 1_000_000 },
  attachmentMaxFileBytes: { min: 1_048_576, max: 104_857_600 },
  attachmentMaxFilesPerMessage: { min: 1, max: 20 },
  /** 0 表示不限制；其余为秒数区间。 */
  runTimeoutSecs: { min: 60, max: 86_400 },
  /** 与后端 MAX_BIN_PATH_CHARS 保持一致。 */
  harnessBinPathChars: 1_024,
} as const;

const PERMISSION_MODES: readonly RoomPermissionMode[] = ["plan", "auto", "full"];

/** 表单使用字符串保存数字输入，提交前统一解析。 */
export interface SettingsDraft {
  defaultRoomPermissionMode: RoomPermissionMode;
  defaultRoomDiscussionRounds: string;
  contextCompressionEnabled: boolean;
  contextBudgetChars: string;
  attachmentMaxFileBytes: string;
  attachmentMaxFilesPerMessage: string;
  runTimeoutSecs: string;
  zcodeBinPath: string;
  codexBinPath: string;
}

export interface SettingsDraftErrors {
  defaultRoomDiscussionRounds?: string;
  contextBudgetChars?: string;
  attachmentMaxFileBytes?: string;
  attachmentMaxFilesPerMessage?: string;
  runTimeoutSecs?: string;
  zcodeBinPath?: string;
  codexBinPath?: string;
}

export type SettingsParseResult =
  | { ok: true; settings: AppSettings }
  | { ok: false; errors: SettingsDraftErrors };

export function settingsToDraft(settings: AppSettings): SettingsDraft {
  return {
    defaultRoomPermissionMode: settings.defaultRoomPermissionMode,
    defaultRoomDiscussionRounds: String(settings.defaultRoomDiscussionRounds),
    contextCompressionEnabled: settings.contextCompressionEnabled,
    contextBudgetChars: String(settings.contextBudgetChars),
    attachmentMaxFileBytes: String(settings.attachmentMaxFileBytes),
    attachmentMaxFilesPerMessage: String(
      settings.attachmentMaxFilesPerMessage,
    ),
    runTimeoutSecs: String(settings.runTimeoutSecs),
    zcodeBinPath: settings.zcodeBinPath ?? "",
    codexBinPath: settings.codexBinPath ?? "",
  };
}

function parseIntInRange(
  raw: string,
  label: string,
  min: number,
  max: number,
): { value?: number; error?: string } {
  const trimmed = raw.trim();
  if (!/^\d+$/.test(trimmed)) {
    return { error: `${label}必须是整数` };
  }
  const value = Number(trimmed);
  if (!Number.isSafeInteger(value) || value < min || value > max) {
    return { error: `${label}必须在 ${min} 到 ${max} 之间` };
  }
  return { value };
}

export function parseSettingsDraft(draft: SettingsDraft): SettingsParseResult {
  const errors: SettingsDraftErrors = {};
  const rounds = parseIntInRange(
    draft.defaultRoomDiscussionRounds,
    "讨论轮数",
    SETTINGS_LIMITS.discussionRounds.min,
    SETTINGS_LIMITS.discussionRounds.max,
  );
  if (rounds.error !== undefined) {
    errors.defaultRoomDiscussionRounds = rounds.error;
  }
  const budget = parseIntInRange(
    draft.contextBudgetChars,
    "上下文预算",
    SETTINGS_LIMITS.contextBudgetChars.min,
    SETTINGS_LIMITS.contextBudgetChars.max,
  );
  if (budget.error !== undefined) {
    errors.contextBudgetChars = budget.error;
  }
  const maxFile = parseIntInRange(
    draft.attachmentMaxFileBytes,
    "单个附件上限",
    SETTINGS_LIMITS.attachmentMaxFileBytes.min,
    SETTINGS_LIMITS.attachmentMaxFileBytes.max,
  );
  if (maxFile.error !== undefined) {
    errors.attachmentMaxFileBytes = maxFile.error;
  }
  const maxFiles = parseIntInRange(
    draft.attachmentMaxFilesPerMessage,
    "单条消息附件数",
    SETTINGS_LIMITS.attachmentMaxFilesPerMessage.min,
    SETTINGS_LIMITS.attachmentMaxFilesPerMessage.max,
  );
  if (maxFiles.error !== undefined) {
    errors.attachmentMaxFilesPerMessage = maxFiles.error;
  }
  const runTimeout = parseRunTimeoutSecs(draft.runTimeoutSecs);
  if (runTimeout.error !== undefined) {
    errors.runTimeoutSecs = runTimeout.error;
  }
  const zcodePath = parseBinPath(draft.zcodeBinPath, "zcode 路径");
  if (zcodePath.error !== undefined) {
    errors.zcodeBinPath = zcodePath.error;
  }
  const codexPath = parseBinPath(draft.codexBinPath, "codex 路径");
  if (codexPath.error !== undefined) {
    errors.codexBinPath = codexPath.error;
  }
  if (Object.keys(errors).length > 0) {
    return { ok: false, errors };
  }
  return {
    ok: true,
    settings: {
      defaultRoomPermissionMode: PERMISSION_MODES.includes(
        draft.defaultRoomPermissionMode,
      )
        ? draft.defaultRoomPermissionMode
        : "auto",
      defaultRoomDiscussionRounds: rounds.value!,
      contextCompressionEnabled: draft.contextCompressionEnabled,
      contextBudgetChars: budget.value!,
      attachmentMaxFileBytes: maxFile.value!,
      attachmentMaxFilesPerMessage: maxFiles.value!,
      runTimeoutSecs: runTimeout.value!,
      zcodeBinPath: zcodePath.value!,
      codexBinPath: codexPath.value!,
    },
  };
}

/** 覆盖路径允许为空（自动检测），非空时限制长度。 */
function parseBinPath(
  raw: string,
  label: string,
): { value?: string; error?: string } {
  const trimmed = raw.trim();
  if (Array.from(trimmed).length > SETTINGS_LIMITS.harnessBinPathChars) {
    return {
      error: `${label}不能超过 ${SETTINGS_LIMITS.harnessBinPathChars} 个字符`,
    };
  }
  return { value: trimmed };
}

/** `0` 表示不限制；其余必须在秒数区间内。 */
function parseRunTimeoutSecs(
  raw: string,
): { value?: number; error?: string } {
  const trimmed = raw.trim();
  if (!/^\d+$/.test(trimmed)) {
    return { error: "运行总时长上限必须是整数" };
  }
  const value = Number(trimmed);
  if (!Number.isSafeInteger(value)) {
    return { error: "运行总时长上限超出安全整数范围" };
  }
  if (
    value !== 0 &&
    (value < SETTINGS_LIMITS.runTimeoutSecs.min ||
      value > SETTINGS_LIMITS.runTimeoutSecs.max)
  ) {
    return {
      error: `运行总时长上限必须为 0（不限制）或在 ${SETTINGS_LIMITS.runTimeoutSecs.min} 到 ${SETTINGS_LIMITS.runTimeoutSecs.max} 之间`,
    };
  }
  return { value };
}

/** 人类可读字节数；用于附件上限输入旁的提示。 */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) {
    return "0 B";
  }
  const units = ["B", "KiB", "MiB", "GiB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  const digits = unit === 0 ? 0 : value >= 10 ? 0 : 1;
  return `${value.toFixed(digits)} ${units[unit]}`;
}

const BINARY_SOURCE_LABELS: Record<HarnessBinarySource, string> = {
  configured: "手动指定",
  protocol: "URL 协议处理器",
  uninstall: "卸载注册表",
  app_paths: "App Paths",
  path: "PATH",
  vendor_default: "厂商默认位置",
  fallback: "自动检测失败",
};

/** 来源标识的中文标签；未知来源原样返回，便于排查后端新增值。 */
export function binarySourceLabel(source: string): string {
  return (
    BINARY_SOURCE_LABELS[source as HarnessBinarySource] ?? source
  );
}

export type BinaryStatusTone = "ok" | "warn";

/** 解析结果的人类可读状态；失败时界面同时展示已尝试来源。 */
export function binaryStatus(status: HarnessBinaryStatus): {
  label: string;
  tone: BinaryStatusTone;
} {
  if (status.exists) {
    return { label: "可用", tone: "ok" };
  }
  if (status.source === "fallback" || status.path.trim() === "") {
    return { label: "未检测到", tone: "warn" };
  }
  return { label: "路径不存在", tone: "warn" };
}
