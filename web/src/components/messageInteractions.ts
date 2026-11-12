//! 房间消息交互的纯函数：引用摘要、反应本地切换、@ 提及切分。
//!
//! 这些函数不触碰 React 或网络，便于单测；组件只负责状态与渲染。

import type { Message, MessageReaction } from "@moon/shared";

/** 与后端 `REACTION_EMOJIS` 保持同序；服务端拒绝调色板外的值。 */
export const REACTION_EMOJIS = ["👍", "❤️", "😄", "🎉", "👀", "🙏"] as const;

export type ReactionEmoji = (typeof REACTION_EMOJIS)[number];

/** 只有落定的普通消息可回复、复制与添加反应。 */
export function isInteractiveMessage(message: Message): boolean {
  return message.status === "final" && message.kind === "reply";
}

/** 引用摘要：折叠空白并按字符截断，避免中文被按 UTF-16 截半。 */
export function summarizeReply(content: string, limit = 80): string {
  const flattened = content.replace(/\s+/g, " ").trim();
  const chars = Array.from(flattened);
  return chars.length <= limit ? flattened : `${chars.slice(0, limit).join("")}…`;
}

/** 乐观切换反应：当前用户已反应则移除/减一，否则新增/加一。 */
export function applyReactionToggle(
  reactions: readonly MessageReaction[],
  emoji: string,
): MessageReaction[] {
  const existing = reactions.find((reaction) => reaction.emoji === emoji);
  if (existing === undefined) {
    return [...reactions, { emoji, count: 1, reactedByMe: true }];
  }
  if (!existing.reactedByMe) {
    return reactions.map((reaction) =>
      reaction.emoji === emoji
        ? {
            ...reaction,
            count: reaction.count + 1,
            reactedByMe: true,
          }
        : reaction,
    );
  }
  if (existing.count > 1) {
    return reactions.map((reaction) =>
      reaction.emoji === emoji
        ? {
            ...reaction,
            count: reaction.count - 1,
            reactedByMe: false,
          }
        : reaction,
    );
  }
  return reactions.filter((reaction) => reaction.emoji !== emoji);
}

export interface MentionSegment {
  text: string;
  mention: boolean;
}

/**
 * 显示用 @ 边界：@ 前须为边界或文本开头，避免邮箱（a@name.com）被误高亮，
 * @ 后须为边界或文本结尾。后端提及解析只校验 @ 后边界，因此这里比触发
 * 规则更保守：宁可不放大，也不把邮箱标成成员。
 */
const MENTION_BOUNDARY = /[\s，。！？、：；,.!?;:()[\]{}【】"'“”‘’]/;

/**
 * 把用户消息切成普通文本与 @成员名 片段。
 *
 * 名称按长度降序匹配，避免 `@小` 抢占 `@小明`；邮箱等非边界 @ 不高亮。
 */
export function splitMentions(
  text: string,
  names: readonly string[],
): MentionSegment[] {
  const unique = [...new Set(names.filter((name) => name.trim() !== ""))].sort(
    (left, right) => right.length - left.length,
  );
  if (unique.length === 0 || !text.includes("@")) {
    return [{ text, mention: false }];
  }

  const segments: MentionSegment[] = [];
  let cursor = 0;
  let index = 0;
  while (index < text.length) {
    if (text[index] !== "@") {
      index += 1;
      continue;
    }
    const before = index === 0 ? null : text[index - 1];
    if (before !== null && !MENTION_BOUNDARY.test(before)) {
      index += 1;
      continue;
    }
    const name = unique.find((candidate) =>
      text.startsWith(candidate, index + 1),
    );
    if (name === undefined) {
      index += 1;
      continue;
    }
    const after = text[index + 1 + name.length];
    if (after !== undefined && !MENTION_BOUNDARY.test(after)) {
      index += 1;
      continue;
    }
    if (index > cursor) {
      segments.push({ text: text.slice(cursor, index), mention: false });
    }
    segments.push({ text: `@${name}`, mention: true });
    index += 1 + name.length;
    cursor = index;
  }
  if (cursor < text.length) {
    segments.push({ text: text.slice(cursor), mention: false });
  }
  return segments;
}
