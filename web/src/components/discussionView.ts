/**
 * 讨论相关的展示文案与判定。
 *
 * 服务端只存状态/立场等枚举值，界面文案集中在这里，避免 RoomPage
 * 内联大量条件分支；保持纯函数，便于单独测试。
 */
import type {
  DiscussionStatus,
  DiscussionStopReason,
  DiscussionStopVote,
  DiscussionStance,
  Message,
} from "@moon/shared";

const STATUS_LABELS: Record<DiscussionStatus, string> = {
  running: "讨论中",
  completed: "讨论完成",
  stopped: "已停止",
  failed: "讨论失败",
};

const STOP_REASON_LABELS: Record<DiscussionStopReason, string> = {
  all_voted_stop: "全员建议结束",
  moderator_decision: "主持人裁决结束",
  max_rounds: "达到最大轮次",
  user_stop: "用户手动停止",
  participant_failed: "参与者运行失败",
  moderator_failed: "主持人裁决失败",
  interrupted: "服务重启中断",
  internal_error: "内部错误",
};

const STANCE_LABELS: Record<DiscussionStance, string> = {
  propose: "提出",
  support: "支持",
  oppose: "反对",
  question: "质疑",
};

const STOP_VOTE_LABELS: Record<DiscussionStopVote, string> = {
  continue: "建议继续",
  stop: "建议结束",
};

const QUOTE_LIMIT = 60;

export function discussionStatusLabel(status: DiscussionStatus): string {
  return STATUS_LABELS[status];
}

/** 失败讨论可续跑；其余状态没有可恢复的断点。 */
export function isResumableDiscussion(status: DiscussionStatus): boolean {
  return status === "failed";
}

export function discussionStopReasonLabel(
  reason: DiscussionStopReason | null,
): string | null {
  return reason === null ? null : STOP_REASON_LABELS[reason];
}

export function discussionStanceLabel(
  stance: DiscussionStance | null,
): string | null {
  return stance === null ? null : STANCE_LABELS[stance];
}

export function discussionStopVoteLabel(
  vote: DiscussionStopVote | null,
): string | null {
  return vote === null ? null : STOP_VOTE_LABELS[vote];
}

export function formatDiscussionRound(
  currentRound: number,
  maxRounds: number,
): string {
  return `第 ${currentRound}/${maxRounds} 轮`;
}

/**
 * 讨论消息的展示类型：
 * - `turn`：参与者发言（含流式/失败占位，立场可能尚未解析）；
 * - `judge`：主持人裁决，无立场；
 * - `system`：讨论开始/结束/失败的系统提示。
 *
 * 只有发送者就是主持人且没有立场时才是裁决：参与者的流式占位与失败
 * 消息同样没有 stance，不能据此误判。
 */
export type DiscussionMessageKind = "turn" | "judge" | "system";

export function discussionMessageKind(
  message: Pick<Message, "senderType" | "senderAgentId" | "discussion">,
  moderatorAgentId: number | null,
): DiscussionMessageKind | null {
  const discussion = message.discussion ?? null;
  if (discussion === null) {
    return null;
  }
  if (message.senderType === "system") {
    return "system";
  }
  if (
    message.senderType === "agent" &&
    moderatorAgentId !== null &&
    message.senderAgentId === moderatorAgentId &&
    discussion.stance === null
  ) {
    return "judge";
  }
  return "turn";
}

/** 引用摘要：压缩换行与连续空白，超长时截断并加省略号。 */
export function summarizeQuote(text: string, limit = QUOTE_LIMIT): string {
  const compact = text.replace(/\s+/g, " ").trim();
  if (compact === "") {
    return "（空消息）";
  }
  if (compact.length <= limit) {
    return compact;
  }
  return `${compact.slice(0, limit).trimEnd()}…`;
}

const CONCLUSION_PREFIX = "讨论结论：";
/** 旧文字墙兜底的最小正文长度；测试用它自证 fixture 确实越过门槛。 */
export const CONCLUSION_WALL_MIN = 240;

/**
 * 侧栏讨论摘要的纯文本预览：去掉 Markdown 标记并限长。
 *
 * 房间侧栏空间有限，完整结论在消息气泡里；这里只保留一句可扫读的摘要。
 */
export function plainDiscussionSummary(text: string, limit = 220): string {
  const plain = text
    .replace(/```[\s\S]*?```/g, " ")
    .replace(/^#{1,6}\s+/gm, "")
    .replace(/\*\*|__/g, "")
    .replace(/^[-*+]\s+/gm, "· ")
    .replace(/\s+/g, " ")
    .trim();
  if (plain.length <= limit) {
    return plain;
  }
  return `${plain.slice(0, limit).trimEnd()}…`;
}

/**
 * 讨论结论的可读性兜底。
 *
 * 旧版主持人把结论写成一整段、只以「；」分隔，Markdown 渲染后是一块
 * 文字墙；新数据由服务端输出 Markdown（含共识/分歧列表），带换行时
 * 原样返回。这里只处理「主持人裁决 + 无换行 + 足够长」的历史消息，
 * 纯展示、不改存储内容。
 */
export function formatDiscussionConclusion(
  content: string,
  kind: DiscussionMessageKind | null,
): string {
  if (kind !== "judge" || !content.startsWith(CONCLUSION_PREFIX)) {
    return content;
  }
  const body = content.slice(CONCLUSION_PREFIX.length).trim();
  if (body.length < CONCLUSION_WALL_MIN || body.includes("\n")) {
    return content;
  }
  const normalized = body.replace(/^结论：/, "");
  const clauses = normalized
    .match(/[^。；！？]+[。；！？]?/g)
    ?.map((clause) => clause.trim())
    .filter((clause) => clause !== "");
  if (clauses === undefined || clauses.length < 3) {
    return content;
  }
  const [first, ...rest] = clauses;
  const lead = first.replace(/[；;]$/, "");
  const bullets = rest.map((clause) => {
    // 以「2)」开头的行在 Markdown 列表项里会被解析成嵌套有序列表，
    // 转义分隔符让它保持普通文本。
    const text = clause
      .replace(/[；;]$/, "")
      .replace(/^(\d+)([.)])\s*/, (_match: string, number: string, separator: string) => {
        return `${number}\\${separator} `;
      });
    return `- ${text}`;
  });
  return `${CONCLUSION_PREFIX}${lead}\n\n${bullets.join("\n")}`;
}

/** 按参与者顺序拼接名字；未知成员回退到 id 占位。 */
export function formatParticipantNames(
  participantIds: readonly number[],
  lookup: (agentId: number) => string | undefined,
): string {
  return participantIds
    .map((agentId) => lookup(agentId) ?? `#${agentId}`)
    .join("、");
}
