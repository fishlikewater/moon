import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { Agent, Message } from "@moon/shared";
import {
  canSendDirectMessage,
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
