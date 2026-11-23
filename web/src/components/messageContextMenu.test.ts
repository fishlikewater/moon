import { describe, expect, it, vi } from "vitest";
import {
  buildMessageLink,
  buildMessageMenuItems,
  type MessageMenuHandlers,
} from "./messageContextMenu";
import type { ContextMenuItem } from "./contextMenuCore";

function handlers(): MessageMenuHandlers {
  return {
    onReply: vi.fn(),
    onCopy: vi.fn(),
    onCopyLink: vi.fn(),
    onToggleReaction: vi.fn(),
    onInspectRun: vi.fn(),
  };
}

function ids(items: ContextMenuItem[]): string[] {
  return items.map((item) => item.id);
}

type MenuItemEntry = Extract<ContextMenuItem, { kind: "item" }>;

function selectItem(items: ContextMenuItem[], id: string): MenuItemEntry {
  const found = items.find((item) => item.id === id);
  if (found === undefined || found.kind !== "item") {
    throw new Error(`missing item ${id}`);
  }
  return found;
}

describe("buildMessageLink", () => {
  it("拼出 origin + pathname + 消息锚点", () => {
    expect(
      buildMessageLink("http://localhost:5173", "/rooms/12", 34),
    ).toBe("http://localhost:5173/rooms/12#message-34");
  });

  it("直聊路径同样适用", () => {
    expect(
      buildMessageLink("https://moon.example", "/chat/room/7", 9),
    ).toBe("https://moon.example/chat/room/7#message-9");
  });

  it("origin 末尾斜杠只保留一个", () => {
    expect(buildMessageLink("http://localhost:5173/", "/rooms/1", 2)).toBe(
      "http://localhost:5173/rooms/1#message-2",
    );
  });
});

describe("buildMessageMenuItems", () => {
  it("房间完整菜单：回复、复制、链接、表情、执行详情", () => {
    const items = buildMessageMenuItems(
      { canReply: true, canReact: true, runId: 42 },
      handlers(),
    );

    expect(ids(items)).toEqual([
      "reply",
      "copy",
      "copy-link",
      "sep-actions",
      "reaction:👍",
      "reaction:❤️",
      "reaction:😄",
      "reaction:🎉",
      "reaction:👀",
      "reaction:🙏",
      "sep-inspect",
      "inspect-run",
    ]);
    expect(items[items.length - 1]?.kind).toBe("item");
  });

  it("直聊只有复制类项与执行详情", () => {
    const items = buildMessageMenuItems(
      { canReply: false, canReact: false, runId: 7 },
      handlers(),
    );

    expect(ids(items)).toEqual(["copy", "copy-link", "sep-inspect", "inspect-run"]);
  });

  it("没有 runId 时不出现执行详情与尾部分隔符", () => {
    const items = buildMessageMenuItems(
      { canReply: true, canReact: true, runId: null },
      handlers(),
    );

    expect(ids(items)).toEqual([
      "reply",
      "copy",
      "copy-link",
      "sep-actions",
      "reaction:👍",
      "reaction:❤️",
      "reaction:😄",
      "reaction:🎉",
      "reaction:👀",
      "reaction:🙏",
    ]);
  });

  it("不可回复/不可反应时只保留复制项", () => {
    const items = buildMessageMenuItems(
      { canReply: false, canReact: false, runId: null },
      handlers(),
    );

    expect(ids(items)).toEqual(["copy", "copy-link"]);
  });

  it("可回复但不可反应时不插入动作分隔符", () => {
    const items = buildMessageMenuItems(
      { canReply: true, canReact: false, runId: null },
      handlers(),
    );

    expect(ids(items)).toEqual(["reply", "copy", "copy-link"]);
  });

  it("所有 id 唯一", () => {
    const items = buildMessageMenuItems(
      { canReply: true, canReact: true, runId: 1 },
      handlers(),
    );

    expect(new Set(ids(items)).size).toBe(items.length);
  });

  it("菜单项回调绑定到对应 handler", () => {
    const spies = handlers();
    const items = buildMessageMenuItems(
      { canReply: true, canReact: true, runId: 1 },
      spies,
    );

    selectItem(items, "reply").onSelect();
    selectItem(items, "copy").onSelect();
    selectItem(items, "copy-link").onSelect();
    selectItem(items, "reaction:👍").onSelect();
    selectItem(items, "inspect-run").onSelect();

    expect(spies.onReply).toHaveBeenCalledTimes(1);
    expect(spies.onCopy).toHaveBeenCalledTimes(1);
    expect(spies.onCopyLink).toHaveBeenCalledTimes(1);
    expect(spies.onToggleReaction).toHaveBeenCalledWith("👍");
    expect(spies.onInspectRun).toHaveBeenCalledTimes(1);
  });
});
