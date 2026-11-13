import type { Message } from "@moon/shared";

/** 超过该字数或行数的智能体最终消息默认折叠。 */
export const COLLAPSE_CHAR_THRESHOLD = 1200;
export const COLLAPSE_LINE_THRESHOLD = 24;

export type MessageFilter = "all" | "moderator" | "members" | "system";

export interface GroupedMessage {
  message: Message;
  /** 与上一条为同一发送者时，后续消息不再重复头像与名称。 */
  isContinuation: boolean;
}

export interface FilterOptions {
  moderatorId: number | null;
  /** 非 null 时只保留该成员的消息，优先于 filter。 */
  agentId: number | null;
}

/** 只有已完成的长智能体回复需要折叠；流式与用户消息保持原样。 */
export function shouldCollapse(message: Message): boolean {
  if (message.senderType !== "agent" || message.status !== "final") {
    return false;
  }
  if (message.content.length > COLLAPSE_CHAR_THRESHOLD) {
    return true;
  }
  return message.content.split(/\r?\n/).length > COLLAPSE_LINE_THRESHOLD;
}

/**
 * 提取前几个标题，作为折叠卡的目录提示；跳过围栏代码块内的注释行。
 */
export function extractHeadings(content: string, limit = 3): string[] {
  const headings: string[] = [];
  let inFence = false;
  for (const line of content.split(/\r?\n/)) {
    const trimmed = line.trim();
    if (/^(```|~~~)/.test(trimmed)) {
      inFence = !inFence;
      continue;
    }
    if (inFence) {
      continue;
    }
    const match = /^#{1,3}\s+(.+)$/.exec(trimmed);
    if (match === null) {
      continue;
    }
    headings.push(match[1]!.trim());
    if (headings.length >= limit) {
      break;
    }
  }
  return headings;
}

/** 只把连续的同发送者、同类型消息标记为续接；系统消息始终独立展示。 */
export function groupMessages(messages: readonly Message[]): GroupedMessage[] {
  return messages.map((message, index) => {
    const previous = messages[index - 1];
    const isContinuation =
      previous !== undefined &&
      message.senderType !== "system" &&
      previous.senderType === message.senderType &&
      previous.senderAgentId === message.senderAgentId &&
      previous.kind === message.kind;
    return { message, isContinuation };
  });
}

/** 过滤只影响展示，不改变消息顺序与底层状态。 */
export function filterMessages(
  messages: readonly Message[],
  filter: MessageFilter,
  options: FilterOptions,
): Message[] {
  return messages.filter((message) => {
    if (options.agentId !== null) {
      return (
        message.senderType === "agent" &&
        message.senderAgentId === options.agentId
      );
    }
    if (filter === "all") {
      return true;
    }
    if (filter === "system") {
      return message.senderType === "system";
    }
    if (message.senderType !== "agent") {
      return false;
    }
    const isModerator =
      options.moderatorId !== null &&
      message.senderAgentId === options.moderatorId;
    return filter === "moderator" ? isModerator : !isModerator;
  });
}

/** 主持人标识类只用于智能体消息，避免用户/系统消息误挂主持人样式。 */
export function moderatorMessageClass(
  senderType: Message["senderType"],
  isModerator: boolean,
): string {
  return senderType === "agent" && isModerator ? " msg-moderator" : "";
}

/** 秒级耗时：不足一小时显示 mm:ss，超过一小时显示 h:mm:ss。 */
export function formatElapsed(ms: number): string {
  const totalSeconds = Math.max(0, Math.floor(ms / 1000));
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;
  const pad = (value: number): string => String(value).padStart(2, "0");
  return hours > 0
    ? `${hours}:${pad(minutes)}:${pad(seconds)}`
    : `${pad(minutes)}:${pad(seconds)}`;
}
