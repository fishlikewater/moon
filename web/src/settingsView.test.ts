import { describe, expect, it } from "vitest";
import {
  formatBytes,
  parseSettingsDraft,
  settingsToDraft,
  SETTINGS_LIMITS,
  type SettingsDraft,
} from "./settingsView";

const VALID: SettingsDraft = {
  defaultRoomPermissionMode: "plan",
  defaultRoomDiscussionRounds: "8",
  contextCompressionEnabled: false,
  contextBudgetChars: "8000",
  attachmentMaxFileBytes: "26214400",
  attachmentMaxFilesPerMessage: "4",
  runTimeoutSecs: "1800",
};

describe("parseSettingsDraft", () => {
  it("解析合法草稿并保留所有字段", () => {
    const result = parseSettingsDraft(VALID);
    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.settings).toEqual({
      defaultRoomPermissionMode: "plan",
      defaultRoomDiscussionRounds: 8,
      contextCompressionEnabled: false,
      contextBudgetChars: 8000,
      attachmentMaxFileBytes: 26_214_400,
      attachmentMaxFilesPerMessage: 4,
      runTimeoutSecs: 1800,
    });
  });

  it("上下文预算上限为一百万", () => {
    const max = parseSettingsDraft({
      ...VALID,
      contextBudgetChars: String(SETTINGS_LIMITS.contextBudgetChars.max),
    });
    expect(max.ok).toBe(true);
    if (max.ok) {
      expect(max.settings.contextBudgetChars).toBe(1_000_000);
    }
    const over = parseSettingsDraft({
      ...VALID,
      contextBudgetChars: "1000001",
    });
    expect(over.ok).toBe(false);
  });

  it("运行时长接受 0 与合法区间，拒绝越界", () => {
    for (const value of ["0", "60", "86400"]) {
      const result = parseSettingsDraft({ ...VALID, runTimeoutSecs: value });
      expect(result.ok).toBe(true);
      if (result.ok) {
        expect(result.settings.runTimeoutSecs).toBe(Number(value));
      }
    }
    for (const value of ["59", "86401", "abc", "-1"]) {
      const result = parseSettingsDraft({ ...VALID, runTimeoutSecs: value });
      expect(result.ok).toBe(false);
      if (!result.ok) {
        expect(result.errors.runTimeoutSecs).toBeTruthy();
      }
    }
  });

  it("拒绝越界与非法数字", () => {
    for (const [key, value, errorKey] of [
      ["defaultRoomDiscussionRounds", "0", "defaultRoomDiscussionRounds"],
      ["defaultRoomDiscussionRounds", "51", "defaultRoomDiscussionRounds"],
      ["contextBudgetChars", "1999", "contextBudgetChars"],
      ["contextBudgetChars", "1000001", "contextBudgetChars"],
      ["attachmentMaxFileBytes", "1024", "attachmentMaxFileBytes"],
      ["attachmentMaxFilesPerMessage", "21", "attachmentMaxFilesPerMessage"],
      ["attachmentMaxFilesPerMessage", "abc", "attachmentMaxFilesPerMessage"],
    ] as const) {
      const result = parseSettingsDraft({ ...VALID, [key]: value });
      expect(result.ok).toBe(false);
      if (!result.ok) {
        expect(result.errors[errorKey]).toBeTruthy();
      }
    }
  });

  it("接受边界值", () => {
    const result = parseSettingsDraft({
      ...VALID,
      defaultRoomDiscussionRounds: String(SETTINGS_LIMITS.discussionRounds.min),
      contextBudgetChars: String(SETTINGS_LIMITS.contextBudgetChars.max),
      attachmentMaxFileBytes: String(
        SETTINGS_LIMITS.attachmentMaxFileBytes.min,
      ),
      attachmentMaxFilesPerMessage: String(
        SETTINGS_LIMITS.attachmentMaxFilesPerMessage.max,
      ),
      runTimeoutSecs: String(SETTINGS_LIMITS.runTimeoutSecs.max),
    });
    expect(result.ok).toBe(true);
  });

  it("服务端设置往返为草稿再解析后不丢值", () => {
    const result = parseSettingsDraft(
      settingsToDraft({
        defaultRoomPermissionMode: "full",
        defaultRoomDiscussionRounds: 9,
        contextCompressionEnabled: true,
        contextBudgetChars: 12_345,
        attachmentMaxFileBytes: 2_000_000,
        attachmentMaxFilesPerMessage: 3,
        runTimeoutSecs: 0,
      }),
    );
    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.settings.defaultRoomPermissionMode).toBe("full");
      expect(result.settings.contextBudgetChars).toBe(12_345);
      expect(result.settings.runTimeoutSecs).toBe(0);
    }
  });
});

describe("formatBytes", () => {
  it("格式化常见字节数", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(1023)).toBe("1023 B");
    expect(formatBytes(1024)).toBe("1.0 KiB");
    expect(formatBytes(26_214_400)).toBe("25 MiB");
  });
});
