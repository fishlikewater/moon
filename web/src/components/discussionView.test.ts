import { describe, expect, it } from "vitest";
import type { Message } from "@moon/shared";
import {
  discussionMessageKind,
  discussionStanceLabel,
  discussionStatusLabel,
  discussionStopReasonLabel,
  discussionStopVoteLabel,
  formatDiscussionConclusion,
  formatDiscussionRound,
  formatParticipantNames,
  isResumableDiscussion,
  plainDiscussionSummary,
  summarizeQuote,
} from "./discussionView";

function message(
  overrides: Partial<Message>,
): Pick<Message, "senderType" | "senderAgentId" | "discussion"> {
  return {
    senderType: "agent",
    senderAgentId: 2,
    discussion: null,
    ...overrides,
  };
}

const MODERATOR_ID = 1;

function emptyDiscussion(round = 1) {
  return {
    discussionId: 1,
    round,
    stance: null,
    replyToMessageId: null,
    stopVote: null,
    stopReason: null,
  };
}

describe("discussionView 文案", () => {
  it("覆盖全部讨论状态与停止原因", () => {
    expect(discussionStatusLabel("running")).toBe("讨论中");
    expect(discussionStatusLabel("completed")).toBe("讨论完成");
    expect(discussionStatusLabel("stopped")).toBe("已停止");
    expect(discussionStatusLabel("failed")).toBe("讨论失败");

    expect(discussionStopReasonLabel(null)).toBeNull();
    expect(discussionStopReasonLabel("all_voted_stop")).toBe("全员建议结束");
    expect(discussionStopReasonLabel("moderator_decision")).toBe("主持人裁决结束");
    expect(discussionStopReasonLabel("max_rounds")).toBe("达到最大轮次");
    expect(discussionStopReasonLabel("user_stop")).toBe("用户手动停止");
    expect(discussionStopReasonLabel("participant_failed")).toBe("参与者运行失败");
    expect(discussionStopReasonLabel("moderator_failed")).toBe("主持人裁决失败");
    expect(discussionStopReasonLabel("interrupted")).toBe("服务重启中断");
    expect(discussionStopReasonLabel("internal_error")).toBe("内部错误");
  });

  it("展示立场与停止建议", () => {
    expect(discussionStanceLabel(null)).toBeNull();
    expect(discussionStanceLabel("propose")).toBe("提出");
    expect(discussionStanceLabel("support")).toBe("支持");
    expect(discussionStanceLabel("oppose")).toBe("反对");
    expect(discussionStanceLabel("question")).toBe("质疑");

    expect(discussionStopVoteLabel(null)).toBeNull();
    expect(discussionStopVoteLabel("continue")).toBe("建议继续");
    expect(discussionStopVoteLabel("stop")).toBe("建议结束");
  });

  it("轮次与参与者名单", () => {
    expect(formatDiscussionRound(2, 3)).toBe("第 2/3 轮");

    const names = new Map<number, string>([
      [1, "前端开发"],
      [2, "代码评审"],
    ]);
    expect(
      formatParticipantNames([1, 2, 9], (agentId) => names.get(agentId)),
    ).toBe("前端开发、代码评审、#9");
  });
});

describe("discussionMessageKind", () => {
  it("区分参与者发言、主持人裁决与系统消息", () => {
    expect(discussionMessageKind(message({}), MODERATOR_ID)).toBeNull();

    expect(
      discussionMessageKind(
        message({
          discussion: {
            ...emptyDiscussion(),
            stance: "oppose",
            stopVote: "continue",
          },
        }),
        MODERATOR_ID,
      ),
    ).toBe("turn");

    // 主持人裁决：发送者就是主持人且没有立场。
    expect(
      discussionMessageKind(
        message({ senderAgentId: MODERATOR_ID, discussion: emptyDiscussion() }),
        MODERATOR_ID,
      ),
    ).toBe("judge");

    expect(
      discussionMessageKind(
        message({
          senderType: "system",
          senderAgentId: null,
          discussion: emptyDiscussion(),
        }),
        MODERATOR_ID,
      ),
    ).toBe("system");
  });

  it("参与者流式/失败消息即使没有立场也不算主持人裁决", () => {
    for (const status of ["streaming", "failed"] as const) {
      expect(
        discussionMessageKind(
          message({ status, discussion: emptyDiscussion() }),
          MODERATOR_ID,
        ),
      ).toBe("turn");
    }
  });

  it("未提供主持人身份时不把无立场智能体消息判为裁决", () => {
    expect(
      discussionMessageKind(
        message({ senderAgentId: MODERATOR_ID, discussion: emptyDiscussion() }),
        null,
      ),
    ).toBe("turn");
  });
});

describe("formatDiscussionConclusion", () => {
  const wall = `讨论结论：结论：${"接口完整；".repeat(60)}最后一句。`;

  it("旧版单段结论按句读拆成短段落", () => {
    const result = formatDiscussionConclusion(wall, "judge");
    expect(result.startsWith("讨论结论：接口完整\n\n- 接口完整")).toBe(true);
    expect(result).toContain("\n\n- ");
    expect(result).not.toContain("结论：结论");
    expect(result.split("\n").length).toBeGreaterThan(10);
  });

  it("编号条目不被 Markdown 当成嵌套有序列表", () => {
    const text = `讨论结论：${"背景；".repeat(90)}1) 第一项；2) 第二项；3) 第三项。`;
    const result = formatDiscussionConclusion(text, "judge");
    expect(result).toContain("- 2\\) 第二项");
    expect(result).toContain("- 3\\) 第三项。");
  });

  it("结构化 Markdown 结论原样返回", () => {
    const text = "讨论结论：采用方案 A\n\n**共识**\n- 先小流量\n\n**分歧**\n- 灰度范围待定";
    expect(formatDiscussionConclusion(text, "judge")).toBe(text);
  });

  it("短结论与非裁决消息不做拆分", () => {
    const short = "讨论结论：采用方案 A。";
    expect(formatDiscussionConclusion(short, "judge")).toBe(short);
    expect(formatDiscussionConclusion(wall, "turn")).toBe(wall);
    expect(formatDiscussionConclusion(wall, null)).toBe(wall);
  });
});

describe("plainDiscussionSummary", () => {
  it("去掉 Markdown 标记并保留可扫读文本", () => {
    const text =
      "结论：采用方案 A。\n\n**共识**\n- 先小流量\n- 灰度两天\n\n**分歧**\n- 范围待定";
    expect(plainDiscussionSummary(text)).toBe(
      "结论：采用方案 A。 共识 · 先小流量 · 灰度两天 分歧 · 范围待定",
    );
  });

  it("超长摘要截断并加省略号", () => {
    const result = plainDiscussionSummary("讨论".repeat(200), 10);
    expect(result).toBe(`${"讨论".repeat(5)}…`);
  });
});

describe("summarizeQuote", () => {
  it("压缩空白并保留短文本", () => {
    expect(summarizeQuote("  第一行\n\n第二行  ")).toBe("第一行 第二行");
  });

  it("空消息给出占位文案", () => {
    expect(summarizeQuote(" \n ")).toBe("（空消息）");
  });

  it("超长文本截断并加省略号", () => {
    const text = "讨论".repeat(40);
    const result = summarizeQuote(text, 10);
    expect(result).toBe(`${"讨论".repeat(5)}…`);
    expect(result.length).toBeLessThanOrEqual(11);
  });
});

describe("isResumableDiscussion", () => {
  it("只有失败讨论可续跑", () => {
    expect(isResumableDiscussion("failed")).toBe(true);
    expect(isResumableDiscussion("running")).toBe(false);
    expect(isResumableDiscussion("completed")).toBe(false);
    expect(isResumableDiscussion("stopped")).toBe(false);
  });
});
