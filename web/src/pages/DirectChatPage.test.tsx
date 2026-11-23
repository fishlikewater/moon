// @vitest-environment jsdom
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
} from "@testing-library/react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Agent, Message, Room } from "@moon/shared";
import {
  canSendDirectMessage,
  DirectChatSideFacts,
  DirectDeleteDialog,
  DirectEmptyState,
  DirectMessage,
} from "./DirectChatPage";

const AGENT: Agent = {
  id: 1,
  name: "小助手",
  persona: "负责回答问题并给出可验收的结论。",
  harness: "fake",
  customHarnessId: null,
  workspaceDir: null,
  modelConfig: null,
  builtinRole: null,
  createdAt: 1,
};

function message(overrides: Partial<Message> = {}): Message {
  return {
    id: 1,
    roomId: 20,
    seq: 1,
    senderType: "agent",
    senderAgentId: 1,
    status: "final",
    kind: "progress",
    runId: null,
    content: "",
    taskId: null,
    createdAt: 1,
    ...overrides,
  };
}

function renderMessage(
  overrides: Partial<Message> = {},
  runId: number | null = null,
): string {
  return renderToStaticMarkup(
    <DirectMessage
      message={message(overrides)}
      agent={AGENT}
      runId={runId}
      onInspectRun={() => {}}
      onLayoutChange={() => {}}
    />,
  );
}

describe("DirectEmptyState", () => {
  it("展示智能体人设与独立沙盒说明", () => {
    const html = renderToStaticMarkup(<DirectEmptyState agent={AGENT} />);

    expect(html).toContain("与 小助手 对话");
    expect(html).toContain(AGENT.persona);
    expect(html).toContain("独立沙盒工作目录");
    expect(html).toContain("无需 @ 成员");
  });
});

function directRoom(): Room {
  return {
    id: 20,
    name: "与 小助手 对话",
    roomType: "direct",
    workspaceDir: "C:/Users/test/.moon/harness-sessions/direct-workspaces/direct-1",
    workspaceDirs: [
      "C:/Users/test/.moon/harness-sessions/direct-workspaces/direct-1",
    ],
    moderatorAgentId: null,
    memberIds: [1],
    replyMode: "mentions",
    discussionMaxRounds: 1,
    permissionMode: "auto",
    createdAt: 1,
    lastActivityAt: null,
  };
}

describe("DirectChatSideFacts", () => {
  it("展示连接、运行、harness、模型、沙盒与权限", () => {
    const html = renderToStaticMarkup(
      <DirectChatSideFacts
        agent={{
          ...AGENT,
          modelConfig: { model: "deepseek-v4", reasoningLevel: "max" },
        }}
        room={directRoom()}
        connected
        runningCount={2}
        permissionSaving={false}
        onChangePermission={() => {}}
      />,
    );

    expect(html).toContain("实时已连接");
    expect(html).toContain("运行中");
    expect(html).toContain("2 个");
    expect(html).toContain("fake");
    expect(html).toContain("deepseek-v4 · max");
    expect(html).toContain("direct-workspaces/direct-1");
    expect(html).toContain("自主模式");
    expect(html).toContain("可读写，危险操作受拦截");
  });

  it("重连中与空闲状态不显示运行数量", () => {
    const html = renderToStaticMarkup(
      <DirectChatSideFacts
        agent={AGENT}
        room={directRoom()}
        connected={false}
        runningCount={0}
        permissionSaving={false}
        onChangePermission={() => {}}
      />,
    );

    expect(html).toContain("重连中…");
    expect(html).toContain("空闲");
    expect(html).not.toContain("0 个");
  });
});

describe("DirectMessage", () => {
  it("用户消息显示「我」与正文", () => {
    const html = renderMessage({
      senderType: "user",
      senderAgentId: null,
      content: "你好",
    });

    expect(html).toContain("我");
    expect(html).toContain("你好");
    expect(html).not.toContain("运行详情");
  });

  it("智能体消息显示名字；有运行时可进入运行详情", () => {
    const html = renderMessage({ content: "收到" }, 7);

    expect(html).toContain("小助手");
    expect(html).toContain("收到");
    expect(html).toContain("运行详情");
  });

  it("失败消息带失败状态标记", () => {
    const html = renderMessage({ status: "failed", content: "运行失败" });

    expect(html).toMatch(/status-pill blocked[^>]*>失败</);
  });

  it("消息操作区提供复制与链接入口", () => {
    const html = renderMessage({ content: "可复制内容" }, 7);

    expect(html).toContain('aria-haspopup="menu"');
    expect(html).toContain(">复制<");
    expect(html).toContain(">链接<");
    expect(html).toContain("运行详情");
  });
});

describe("DirectMessage 复制操作", () => {
  const originalClipboard = Object.getOwnPropertyDescriptor(
    navigator,
    "clipboard",
  );

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
    if (originalClipboard === undefined) {
      Reflect.deleteProperty(navigator, "clipboard");
    } else {
      Object.defineProperty(navigator, "clipboard", originalClipboard);
    }
    window.history.replaceState({}, "", "/");
  });

  function installClipboard(writeText: (text: string) => Promise<void>): void {
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText },
    });
  }

  function renderInteractive(): void {
    render(
      <DirectMessage
        message={message({ id: 43, content: "直聊复制内容" })}
        agent={AGENT}
        runId={null}
        onInspectRun={() => {}}
        onLayoutChange={() => {}}
      />,
    );
  }

  it("点击链接把当前消息 URL 写入剪贴板并反馈", async () => {
    window.history.replaceState({}, "", "/chat/room/20");
    const writeText = vi.fn<(text: string) => Promise<void>>().mockResolvedValue();
    installClipboard(writeText);
    renderInteractive();

    fireEvent.click(screen.getByRole("button", { name: "链接" }));

    expect(await screen.findByRole("button", { name: "已复制链接" })).toBeTruthy();
    expect(writeText).toHaveBeenCalledWith(
      "http://localhost:3000/chat/room/20#message-43",
    );
  });

  it("点击复制写入消息正文", async () => {
    const writeText = vi.fn<(text: string) => Promise<void>>().mockResolvedValue();
    installClipboard(writeText);
    renderInteractive();

    fireEvent.click(screen.getByRole("button", { name: "复制" }));

    expect(await screen.findByRole("button", { name: "已复制" })).toBeTruthy();
    expect(writeText).toHaveBeenCalledWith("直聊复制内容");
  });

  it("剪贴板失败时按钮反馈失败", async () => {
    const writeText = vi
      .fn<(text: string) => Promise<void>>()
      .mockRejectedValue(new Error("denied"));
    installClipboard(writeText);
    renderInteractive();

    fireEvent.click(screen.getByRole("button", { name: "链接" }));

    expect(await screen.findByRole("button", { name: "链接失败" })).toBeTruthy();
  });

  it("反馈 1.5 秒后恢复，卸载后计时器不再触发", async () => {
    vi.useFakeTimers();
    try {
      const writeText = vi
        .fn<(text: string) => Promise<void>>()
        .mockResolvedValue();
      installClipboard(writeText);
      const view = render(
        <DirectMessage
          message={message({ id: 44, content: "计时反馈" })}
          agent={AGENT}
          runId={null}
          onInspectRun={() => {}}
          onLayoutChange={() => {}}
        />,
      );

      await act(async () => {
        fireEvent.click(screen.getByRole("button", { name: "复制" }));
      });
      expect(screen.getByRole("button", { name: "已复制" })).toBeTruthy();

      act(() => {
        vi.advanceTimersByTime(1500);
      });
      expect(screen.getByRole("button", { name: "复制" })).toBeTruthy();

      await act(async () => {
        fireEvent.click(screen.getByRole("button", { name: "复制" }));
      });
      view.unmount();
      expect(() => {
        act(() => {
          vi.advanceTimersByTime(2000);
        });
      }).not.toThrow();
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("canSendDirectMessage", () => {
  it("空白草稿且无附件时禁用", () => {
    expect(canSendDirectMessage("", 0, false)).toBe(false);
    expect(canSendDirectMessage("   ", 0, false)).toBe(false);
  });

  it("有正文或附件时可用；发送中始终禁用", () => {
    expect(canSendDirectMessage("hi", 0, false)).toBe(true);
    expect(canSendDirectMessage("", 1, false)).toBe(true);
    expect(canSendDirectMessage("hi", 1, true)).toBe(false);
  });
});

describe("DirectDeleteDialog", () => {
  function render(overrides: {
    runningCount?: number;
    deleting?: boolean;
    error?: string | null;
  } = {}): string {
    return renderToStaticMarkup(
      <DirectDeleteDialog
        agentName="小助手"
        runningCount={overrides.runningCount ?? 0}
        deleting={overrides.deleting ?? false}
        error={overrides.error ?? null}
        onCancel={() => {}}
        onConfirm={() => {}}
      />,
    );
  }

  it("说明删除后果并保留工作区文件", () => {
    const html = render();

    expect(html).toContain("删除对话");
    expect(html).toContain("消息、运行记录与会话都会删除，无法恢复");
    expect(html).toContain("工作区文件保留");
    expect(html).toMatch(/<button[^>]*>确认删除<\/button>/);
  });

  it("运行中禁用确认并提示等待", () => {
    const html = render({ runningCount: 1 });

    expect(html).toContain("有运行中的回复");
    expect(html).toMatch(/<button[^>]*disabled=""[^>]*>确认删除<\/button>/);
  });

  it("删除中与失败原因都在弹层内反馈", () => {
    const deleting = render({ deleting: true });
    expect(deleting).toContain("删除中…");

    const failed = render({ error: "房间有运行中的回复" });
    expect(failed).toMatch(/role="alert"[^>]*>房间有运行中的回复</);
  });
});
