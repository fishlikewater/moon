import type { Message } from "@moon/shared";
import { describe, expect, it } from "vitest";
import {
  extractHeadings,
  filterMessages,
  formatElapsed,
  groupMessages,
  moderatorMessageClass,
  shouldCollapse,
} from "./messageView";

function message(overrides: Partial<Message> = {}): Message {
  return {
    id: 1,
    roomId: 1,
    seq: 1,
    senderType: "agent",
    senderAgentId: 10,
    status: "final",
    kind: "reply",
    runId: null,
    content: "短回复",
    taskId: null,
    createdAt: 1_700_000_000_000,
    ...overrides,
  };
}

describe("shouldCollapse", () => {
  it("只折叠超长的已完成智能体消息", () => {
    expect(shouldCollapse(message({ content: "短回复" }))).toBe(false);
    expect(
      shouldCollapse(message({ content: "长".repeat(1201) })),
    ).toBe(true);
    expect(
      shouldCollapse(
        message({
          status: "streaming",
          content: "长".repeat(2000),
        }),
      ),
    ).toBe(false);
    expect(
      shouldCollapse(
        message({
          senderType: "user",
          senderAgentId: null,
          content: "长".repeat(2000),
        }),
      ),
    ).toBe(false);
  });

  it("行数超过阈值也会折叠", () => {
    expect(
      shouldCollapse(message({ content: Array.from({ length: 25 }, () => "行").join("\n") })),
    ).toBe(true);
  });
});

describe("extractHeadings", () => {
  it("提取前三个一至三级标题", () => {
    expect(
      extractHeadings("# 一\n正文\n## 二\n### 三\n#### 不取"),
    ).toEqual(["一", "二", "三"]);
  });

  it("跳过围栏代码块里的井号注释", () => {
    expect(
      extractHeadings("# 真标题\n```bash\n# 这是注释\n```\n## 第二个"),
    ).toEqual(["真标题", "第二个"]);
  });
});

describe("groupMessages", () => {
  it("连续同发送者标记为续接，切换发送者后重置", () => {
    const items = groupMessages([
      message({ id: 1, seq: 1, senderAgentId: 10 }),
      message({ id: 2, seq: 2, senderAgentId: 10 }),
      message({ id: 3, seq: 3, senderAgentId: 11 }),
      message({ id: 4, seq: 4, senderType: "system", senderAgentId: null }),
      message({ id: 5, seq: 5, senderType: "system", senderAgentId: null }),
    ]);
    expect(items.map((item) => item.isContinuation)).toEqual([
      false,
      true,
      false,
      false,
      false,
    ]);
  });

  it("progress 与 reply 不互为续接", () => {
    const items = groupMessages([
      message({ id: 1, seq: 1, senderAgentId: 10, kind: "progress" }),
      message({ id: 2, seq: 2, senderAgentId: 10, kind: "progress" }),
      message({ id: 3, seq: 3, senderAgentId: 10, kind: "reply" }),
      message({ id: 4, seq: 4, senderAgentId: 10, kind: "reply" }),
    ]);

    expect(items.map((item) => item.isContinuation)).toEqual([
      false,
      true,
      false,
      true,
    ]);
  });
});

describe("filterMessages", () => {
  const messages = [
    message({ id: 1, seq: 1, senderAgentId: 10 }),
    message({ id: 2, seq: 2, senderAgentId: 11 }),
    message({ id: 3, seq: 3, senderType: "user", senderAgentId: null }),
    message({ id: 4, seq: 4, senderType: "system", senderAgentId: null }),
  ];

  it("按主持人、成员与系统过滤", () => {
    expect(
      filterMessages(messages, "moderator", {
        moderatorId: 10,
        agentId: null,
      }).map((item) => item.id),
    ).toEqual([1]);
    expect(
      filterMessages(messages, "members", {
        moderatorId: 10,
        agentId: null,
      }).map((item) => item.id),
    ).toEqual([2]);
    expect(
      filterMessages(messages, "system", {
        moderatorId: 10,
        agentId: null,
      }).map((item) => item.id),
    ).toEqual([4]);
  });

  it("成员过滤优先于分类过滤", () => {
    expect(
      filterMessages(messages, "system", {
        moderatorId: 10,
        agentId: 11,
      }).map((item) => item.id),
    ).toEqual([2]);
  });

  it("阶段消息随消息分类一起保留", () => {
    const progress = message({
      id: 5,
      seq: 5,
      kind: "progress",
      senderAgentId: 11,
    });
    const withProgress = [...messages, progress];
    const options = { moderatorId: 10, agentId: null };

    expect(
      filterMessages(withProgress, "all", options).map((item) => item.id),
    ).toEqual([1, 2, 3, 4, 5]);
    expect(
      filterMessages(withProgress, "members", options).map((item) => item.id),
    ).toEqual([2, 5]);
    expect(
      filterMessages(withProgress, "system", options).map((item) => item.id),
    ).toEqual([4]);
  });

  it("没有匹配时返回空数组", () => {
    expect(
      filterMessages(messages, "members", {
        moderatorId: 10,
        agentId: 99,
      }),
    ).toEqual([]);
  });
});

describe("formatElapsed", () => {
  it("按秒格式化并处理小时", () => {
    expect(formatElapsed(0)).toBe("00:00");
    expect(formatElapsed(65_000)).toBe("01:05");
    expect(formatElapsed(3_725_000)).toBe("1:02:05");
    expect(formatElapsed(-1000)).toBe("00:00");
  });
});

describe("moderatorMessageClass", () => {
  it("只给主持人智能体消息加标识类", () => {
    expect(moderatorMessageClass("agent", true)).toBe(" msg-moderator");
    expect(moderatorMessageClass("agent", false)).toBe("");
    expect(moderatorMessageClass("user", true)).toBe("");
    expect(moderatorMessageClass("system", true)).toBe("");
  });
});
