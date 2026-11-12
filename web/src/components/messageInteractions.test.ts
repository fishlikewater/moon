import { describe, expect, it } from "vitest";
import type { Message, MessageReaction } from "@moon/shared";
import {
  applyReactionToggle,
  isInteractiveMessage,
  summarizeReply,
  splitMentions,
} from "./messageInteractions";

function message(overrides: Partial<Message> = {}): Message {
  return {
    id: 1,
    roomId: 1,
    seq: 1,
    senderType: "user",
    senderAgentId: null,
    status: "final",
    kind: "reply",
    runId: null,
    content: "内容",
    taskId: null,
    createdAt: 0,
    ...overrides,
  };
}

describe("messageInteractions", () => {
  it("引用摘要折叠空白并按字符截断", () => {
    expect(summarizeReply("  第一行\n\n第二行  ")).toBe("第一行 第二行");
    expect(summarizeReply("一二三四五", 3)).toBe("一二三…");
    const emojiText = "😀".repeat(81);
    expect(Array.from(summarizeReply(emojiText)).length).toBe(81);
    expect(summarizeReply(emojiText).endsWith("…")).toBe(true);
  });

  it("反应切换新增、移除并保留其他 emoji", () => {
    const other: MessageReaction = {
      emoji: "🎉",
      count: 2,
      reactedByMe: false,
    };
    const added = applyReactionToggle([other], "👍");
    expect(added).toEqual([
      other,
      { emoji: "👍", count: 1, reactedByMe: true },
    ]);

    const removed = applyReactionToggle(added, "👍");
    expect(removed).toEqual([other]);
  });

  it("他人已有反应时切换只增减计数", () => {
    const base: MessageReaction[] = [
      { emoji: "👍", count: 3, reactedByMe: false },
    ];
    expect(applyReactionToggle(base, "👍")).toEqual([
      { emoji: "👍", count: 4, reactedByMe: true },
    ]);
    expect(
      applyReactionToggle([{ emoji: "👍", count: 3, reactedByMe: true }], "👍"),
    ).toEqual([{ emoji: "👍", count: 2, reactedByMe: false }]);
  });

  it("@ 提及按最长名称匹配并保留普通文本", () => {
    const segments = splitMentions("请 @小明 和 @小 看一下", ["小", "小明"]);
    expect(segments).toEqual([
      { text: "请 ", mention: false },
      { text: "@小明", mention: true },
      { text: " 和 ", mention: false },
      { text: "@小", mention: true },
      { text: " 看一下", mention: false },
    ]);
  });

  it("@ 提及要求边界，邮箱不高亮", () => {
    expect(splitMentions("a@小明.com", ["小明"])).toEqual([
      { text: "a@小明.com", mention: false },
    ]);
    expect(splitMentions("你好，@小明。", ["小明"])).toEqual([
      { text: "你好，", mention: false },
      { text: "@小明", mention: true },
      { text: "。", mention: false },
    ]);
    expect(splitMentions("没有成员名", [])).toEqual([
      { text: "没有成员名", mention: false },
    ]);
  });

  it("只有落定的普通消息可交互", () => {
    expect(isInteractiveMessage(message())).toBe(true);
    expect(isInteractiveMessage(message({ status: "streaming" }))).toBe(false);
    expect(isInteractiveMessage(message({ status: "failed" }))).toBe(false);
    expect(isInteractiveMessage(message({ kind: "progress" }))).toBe(false);
  });
});
