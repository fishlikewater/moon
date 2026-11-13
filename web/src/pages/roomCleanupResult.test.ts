import type { MessageCleanupResult } from "@moon/shared";
import { describe, expect, it } from "vitest";
import { formatCleanupResult } from "./roomCleanupResult";

function makeResult(
  overrides: Partial<MessageCleanupResult> = {},
): MessageCleanupResult {
  return {
    deleted: 0,
    hidden: 0,
    artifactsDeleted: 0,
    attachmentsDeleted: 0,
    ...overrides,
  };
}

describe("formatCleanupResult", () => {
  it("删除与隐藏都为零时提示无可清理", () => {
    expect(formatCleanupResult(makeResult())).toBe("没有可清理的聊天记录");
  });

  it("只隐藏受保护消息时说明记录本身保留", () => {
    expect(formatCleanupResult(makeResult({ hidden: 3 }))).toBe(
      "已从聊天窗口隐藏 3 条讨论/任务/委派记录（记录本身保留）",
    );
  });

  it("同时报告物理删除与窗口隐藏", () => {
    expect(formatCleanupResult(makeResult({ deleted: 2, hidden: 5 }))).toBe(
      "已清理 2 条聊天记录，已从聊天窗口隐藏 5 条讨论/任务/委派记录（记录本身保留）",
    );
  });

  it("追加归档与附件计数", () => {
    expect(
      formatCleanupResult(
        makeResult({
          deleted: 1,
          hidden: 4,
          artifactsDeleted: 2,
          attachmentsDeleted: 3,
        }),
      ),
    ).toBe(
      "已清理 1 条聊天记录，已从聊天窗口隐藏 4 条讨论/任务/委派记录（记录本身保留），同时删除 2 个归档文件、3 个附件",
    );
  });

  it("只有删除时保持原有文案", () => {
    expect(
      formatCleanupResult(makeResult({ deleted: 6, artifactsDeleted: 1 })),
    ).toBe("已清理 6 条聊天记录，同时删除 1 个归档文件");
  });
});
