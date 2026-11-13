import type { Message } from "@moon/shared";
import { describe, expect, it } from "vitest";
import {
  cleanupPreview,
  isPureChatMessage,
  parseKeepRecent,
} from "./roomCleanup";

function makeMessage(overrides: Partial<Message> = {}): Message {
  return {
    id: 1,
    roomId: 1,
    seq: 1,
    senderType: "user",
    senderAgentId: null,
    status: "final",
    kind: "reply",
    runId: null,
    content: "消息",
    taskId: null,
    createdAt: 1_700_000_000_000,
    ...overrides,
  };
}

describe("isPureChatMessage", () => {
  it("只把无任务、无讨论、无委派的消息视为纯聊天", () => {
    expect(isPureChatMessage(makeMessage())).toBe(true);
    expect(isPureChatMessage(makeMessage({ taskId: 3 }))).toBe(false);
    expect(
      isPureChatMessage(
        makeMessage({
          discussion: {
            discussionId: 1,
            round: 1,
            stance: null,
            replyToMessageId: null,
            stopVote: null,
            stopReason: null,
          },
        }),
      ),
    ).toBe(false);
    expect(
      isPureChatMessage(
        makeMessage({
          collaboration: { batchId: 1, round: 1, role: "assignment" },
        }),
      ),
    ).toBe(false);
  });
});

describe("cleanupPreview", () => {
  it("0 表示清理全部聊天窗口历史，受保护消息计入隐藏", () => {
    const messages = [
      makeMessage({ id: 1 }),
      makeMessage({ id: 2, taskId: 1 }),
      makeMessage({ id: 3 }),
    ];
    expect(cleanupPreview(messages, 0)).toEqual({
      eligible: 2,
      deleting: 2,
      keeping: 0,
      hidden: 1,
    });
  });

  it("keepRecent 只保留最近 N 条纯聊天消息", () => {
    const messages = [
      makeMessage({ id: 1 }),
      makeMessage({ id: 2, taskId: 1 }),
      makeMessage({ id: 3 }),
    ];
    expect(cleanupPreview(messages, 1)).toEqual({
      eligible: 2,
      deleting: 1,
      keeping: 1,
      hidden: 1,
    });
  });

  it("没有纯聊天时只有隐藏计数，按钮仍可执行清理", () => {
    const messages = [
      makeMessage({ id: 1, taskId: 3 }),
      makeMessage({
        id: 2,
        discussion: {
          discussionId: 1,
          round: 1,
          stance: null,
          replyToMessageId: null,
          stopVote: null,
          stopReason: null,
        },
      }),
    ];
    expect(cleanupPreview(messages, 0)).toEqual({
      eligible: 0,
      deleting: 0,
      keeping: 0,
      hidden: 2,
    });
  });

  it("保留数超过总数时不删除也不隐藏", () => {
    expect(cleanupPreview([makeMessage({ id: 1 })], 10)).toEqual({
      eligible: 1,
      deleting: 0,
      keeping: 1,
      hidden: 0,
    });
  });
});

describe("parseKeepRecent", () => {
  it("空串按 0 处理", () => {
    expect(parseKeepRecent("")).toBe(0);
    expect(parseKeepRecent("  ")).toBe(0);
  });

  it("接受 0 到 5000 的整数", () => {
    expect(parseKeepRecent("0")).toBe(0);
    expect(parseKeepRecent("12")).toBe(12);
    expect(parseKeepRecent("5000")).toBe(5000);
  });

  it("拒绝负数、小数、非数字与越界值", () => {
    expect(parseKeepRecent("-1")).toBeNull();
    expect(parseKeepRecent("1.5")).toBeNull();
    expect(parseKeepRecent("abc")).toBeNull();
    expect(parseKeepRecent("5001")).toBeNull();
  });
});
