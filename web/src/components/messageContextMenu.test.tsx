// @vitest-environment jsdom
import {
  cleanup,
  createEvent,
  fireEvent,
  render,
  screen,
} from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ContextMenu, useContextMenu } from "./ContextMenu";
import {
  buildMessageMenuItems,
  type MessageMenuHandlers,
  type MessageMenuOptions,
} from "./messageContextMenu";

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

function handlers(): MessageMenuHandlers {
  return {
    onReply: vi.fn(),
    onCopy: vi.fn(),
    onCopyLink: vi.fn(),
    onToggleReaction: vi.fn(),
    onInspectRun: vi.fn(),
  };
}

function Harness({
  options,
  spies,
}: {
  options: MessageMenuOptions;
  spies: MessageMenuHandlers;
}) {
  const menu = useContextMenu();
  const items = buildMessageMenuItems(options, spies);
  return (
    <div>
      <article
        aria-label="消息"
        onContextMenu={(event) => {
          menu.openAt(event);
        }}
      >
        消息内容
      </article>
      <ContextMenu
        open={menu.open}
        position={menu.position}
        items={items}
        onClose={menu.close}
        ariaLabel="消息操作"
      />
    </div>
  );
}

function openOnMessage() {
  const article = screen.getByLabelText("消息");
  fireEvent.contextMenu(article, { clientX: 20, clientY: 30 });
}

describe("消息右键菜单接线", () => {
  it("房间菜单点击回复、表情与执行详情分别触发回调", () => {
    const spies = handlers();
    render(
      <Harness
        options={{ canReply: true, canReact: true, runId: 5 }}
        spies={spies}
      />,
    );

    openOnMessage();
    fireEvent.click(screen.getByRole("menuitem", { name: "回复" }));
    expect(spies.onReply).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("menu")).toBeNull();

    openOnMessage();
    fireEvent.click(screen.getByRole("menuitem", { name: "👍" }));
    expect(spies.onToggleReaction).toHaveBeenCalledWith("👍");

    openOnMessage();
    fireEvent.click(screen.getByRole("menuitem", { name: "执行详情" }));
    expect(spies.onInspectRun).toHaveBeenCalledTimes(1);
  });

  it("直聊菜单只有复制类项与执行详情", () => {
    const spies = handlers();
    render(
      <Harness
        options={{ canReply: false, canReact: false, runId: 8 }}
        spies={spies}
      />,
    );

    openOnMessage();
    expect(screen.queryByRole("menuitem", { name: "回复" })).toBeNull();
    expect(screen.queryByRole("menuitem", { name: "👍" })).toBeNull();

    fireEvent.click(screen.getByRole("menuitem", { name: "复制文本" }));
    expect(spies.onCopy).toHaveBeenCalledTimes(1);

    openOnMessage();
    fireEvent.click(screen.getByRole("menuitem", { name: "复制消息链接" }));
    expect(spies.onCopyLink).toHaveBeenCalledTimes(1);

    openOnMessage();
    fireEvent.click(screen.getByRole("menuitem", { name: "执行详情" }));
    expect(spies.onInspectRun).toHaveBeenCalledTimes(1);
  });

  it("消息内存在文本选区时保留原生菜单", () => {
    const spies = handlers();
    render(
      <Harness
        options={{ canReply: true, canReact: true, runId: null }}
        spies={spies}
      />,
    );
    const article = screen.getByLabelText("消息");
    const range = document.createRange();
    range.selectNodeContents(article);
    const selection = window.getSelection();
    selection?.removeAllRanges();
    selection?.addRange(range);

    try {
      const event = createEvent.contextMenu(article, {
        clientX: 20,
        clientY: 30,
      });
      fireEvent(article, event);

      expect(event.defaultPrevented).toBe(false);
      expect(screen.queryByRole("menu")).toBeNull();
    } finally {
      selection?.removeAllRanges();
    }
  });

  it("Esc 关闭消息菜单", () => {
    render(
      <Harness
        options={{ canReply: true, canReact: true, runId: null }}
        spies={handlers()}
      />,
    );
    openOnMessage();
    expect(screen.getByRole("menu", { name: "消息操作" })).not.toBeNull();

    fireEvent.keyDown(document.body, { key: "Escape" });

    expect(screen.queryByRole("menu")).toBeNull();
  });
});
