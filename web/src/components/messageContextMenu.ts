import type { ContextMenuItem } from "./contextMenuCore";
import { REACTION_EMOJIS } from "./messageInteractions";

export type MessageMenuOptions = {
  canReply: boolean;
  canReact: boolean;
  runId: number | null;
};

export type MessageMenuHandlers = {
  onReply: () => void;
  onCopy: () => void;
  onCopyLink: () => void;
  onToggleReaction: (emoji: string) => void;
  onInspectRun: () => void;
};

/** 消息锚点与消息 article 的 `id` 保持一致：`#message-<id>`。 */
export function buildMessageLink(
  origin: string,
  pathname: string,
  messageId: number,
): string {
  const base = origin.endsWith("/") ? origin.slice(0, -1) : origin;
  return `${base}${pathname}#message-${messageId}`;
}

/**
 * 房间与直聊共用同一菜单构造函数：房间多出回复与表情，直聊只有复制类项；
 * 执行详情取决于 runId，分隔符只在后面确实有项时加入。
 */
export function buildMessageMenuItems(
  options: MessageMenuOptions,
  handlers: MessageMenuHandlers,
): ContextMenuItem[] {
  const items: ContextMenuItem[] = [];
  if (options.canReply) {
    items.push({
      kind: "item",
      id: "reply",
      label: "回复",
      onSelect: handlers.onReply,
    });
  }
  items.push({
    kind: "item",
    id: "copy",
    label: "复制文本",
    onSelect: handlers.onCopy,
  });
  items.push({
    kind: "item",
    id: "copy-link",
    label: "复制消息链接",
    onSelect: handlers.onCopyLink,
  });
  if (options.canReact) {
    items.push({ kind: "separator", id: "sep-actions" });
    for (const emoji of REACTION_EMOJIS) {
      items.push({
        kind: "item",
        id: `reaction:${emoji}`,
        label: emoji,
        onSelect: () => handlers.onToggleReaction(emoji),
      });
    }
  }
  if (options.runId !== null) {
    items.push({ kind: "separator", id: "sep-inspect" });
    items.push({
      kind: "item",
      id: "inspect-run",
      label: "执行详情",
      onSelect: handlers.onInspectRun,
    });
  }
  return items;
}
