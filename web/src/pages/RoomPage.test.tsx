// @vitest-environment jsdom
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Message } from "@moon/shared";
import { MessageCard } from "./RoomPage";

function message(overrides: Partial<Message> = {}): Message {
  return {
    id: 301,
    roomId: 1,
    seq: 1,
    senderType: "user",
    senderAgentId: null,
    status: "final",
    kind: "reply",
    runId: null,
    content: "房间消息",
    taskId: null,
    createdAt: 1,
    ...overrides,
  };
}

function renderCard(
  overrides: Partial<Message> = {},
  runId: number | null = null,
) {
  const handlers = {
    onInspectRun: vi.fn(),
    onReply: vi.fn(),
    onCopy: vi.fn(),
    onCopyLink: vi.fn(),
    onToggleReaction: vi.fn(),
    onJumpToMessage: vi.fn(),
    onLayoutChange: vi.fn(),
  };
  render(
    <MessageCard
      message={message(overrides)}
      name="我"
      isContinuation={false}
      isModerator={false}
      moderatorId={null}
      agentNameById={new Map()}
      quoted={undefined}
      quotedName={null}
      replyToId={null}
      mentionNames={[]}
      durationMs={null}
      runId={runId}
      onInspectRun={handlers.onInspectRun}
      onReply={handlers.onReply}
      onCopy={handlers.onCopy}
      onCopyLink={handlers.onCopyLink}
      onToggleReaction={handlers.onToggleReaction}
      onJumpToMessage={handlers.onJumpToMessage}
      copied={false}
      copiedLink={false}
      reactionBusy={false}
      flash={false}
      onLayoutChange={handlers.onLayoutChange}
    />,
  );
  const article = document.getElementById("message-301");
  if (article === null) {
    throw new Error("消息 article 未渲染");
  }
  return { article, ...handlers };
}

async function openMenu(article: HTMLElement): Promise<HTMLElement> {
  const notPrevented = fireEvent.contextMenu(article, {
    clientX: 120,
    clientY: 160,
  });
  expect(notPrevented).toBe(false);
  return screen.findByRole("menu", { name: "消息操作" });
}

describe("MessageCard 右键菜单", () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("可交互消息暴露菜单语义并按 AC-002 顺序渲染菜单项", async () => {
    const { article } = renderCard({}, 7);

    expect(article.getAttribute("aria-haspopup")).toBe("menu");

    const menu = await openMenu(article);
    const labels = within(menu)
      .getAllByRole("menuitem")
      .map((item) => item.textContent?.trim());

    expect(labels).toEqual([
      "回复",
      "复制文本",
      "复制消息链接",
      "👍",
      "❤️",
      "😄",
      "🎉",
      "👀",
      "🙏",
      "执行详情",
    ]);
    expect(within(menu).getAllByRole("separator")).toHaveLength(2);
  });

  it("菜单项调用与消息绑定的房间回调", async () => {
    const {
      article,
      onReply,
      onCopy,
      onCopyLink,
      onToggleReaction,
      onInspectRun,
    } = renderCard({}, 7);

    await openMenu(article);
    fireEvent.click(screen.getByRole("menuitem", { name: "回复" }));
    expect(onReply).toHaveBeenCalledWith(
      expect.objectContaining({ id: 301 }),
    );

    await waitFor(() => expect(screen.queryByRole("menu")).toBeNull());
    await openMenu(article);
    fireEvent.click(screen.getByRole("menuitem", { name: "复制文本" }));
    expect(onCopy).toHaveBeenCalledWith(
      expect.objectContaining({ id: 301 }),
    );

    await waitFor(() => expect(screen.queryByRole("menu")).toBeNull());
    await openMenu(article);
    fireEvent.click(screen.getByRole("menuitem", { name: "复制消息链接" }));
    expect(onCopyLink).toHaveBeenCalledWith(
      expect.objectContaining({ id: 301 }),
    );

    await waitFor(() => expect(screen.queryByRole("menu")).toBeNull());
    await openMenu(article);
    fireEvent.click(screen.getByRole("menuitem", { name: "👍" }));
    expect(onToggleReaction).toHaveBeenCalledWith(301, "👍");

    await waitFor(() => expect(screen.queryByRole("menu")).toBeNull());
    await openMenu(article);
    fireEvent.click(screen.getByRole("menuitem", { name: "执行详情" }));
    expect(onInspectRun).toHaveBeenCalledWith(7);
  });

  it("流式消息不接管右键，也不声明菜单语义", async () => {
    const { article } = renderCard({ status: "streaming" });

    expect(article.getAttribute("aria-haspopup")).toBeNull();
    const notPrevented = fireEvent.contextMenu(article, {
      clientX: 10,
      clientY: 10,
    });

    expect(notPrevented).toBe(true);
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("阶段总结消息不接管右键", () => {
    const { article } = renderCard({ kind: "progress" });

    expect(article.getAttribute("aria-haspopup")).toBeNull();
    const notPrevented = fireEvent.contextMenu(article, {
      clientX: 10,
      clientY: 10,
    });

    expect(notPrevented).toBe(true);
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("runId 为空时菜单没有执行详情", async () => {
    const { article } = renderCard({}, null);

    const menu = await openMenu(article);

    expect(
      within(menu).queryByRole("menuitem", { name: "执行详情" }),
    ).toBeNull();
    expect(within(menu).getAllByRole("separator")).toHaveLength(1);
  });
});
