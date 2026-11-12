import type { Message } from "@moon/shared";

/** 前端与服务端共享的 keepRecent 上限副本（shared 是 CJS 产物，不加载运行时）。 */
export const MAX_KEEP_RECENT = 5_000;

/** 纯聊天消息：没有任务、讨论或委派关联。 */
export function isPureChatMessage(message: Message): boolean {
  return (
    message.taskId === null &&
    message.discussion == null &&
    message.collaboration == null
  );
}

export interface CleanupPreview {
  /** 可参与清理的纯聊天消息数 */
  eligible: number;
  /** 按当前输入预计删除的条数 */
  deleting: number;
  /** 预计保留的条数 */
  keeping: number;
}

/**
 * 计算清理预览。
 *
 * 服务端还会额外保护被委派批次/讨论外键引用的消息，因此这里显示的是
 * 「上限估计」；真正删除数量以接口返回为准。
 */
export function cleanupPreview(
  messages: readonly Message[],
  keepRecent: number,
): CleanupPreview {
  const eligible = messages.filter(isPureChatMessage).length;
  const keep = Math.min(Math.max(keepRecent, 0), eligible);
  return { eligible, deleting: eligible - keep, keeping: keep };
}

/** 解析输入框：空串按 0；非法或越界返回 null。 */
export function parseKeepRecent(value: string): number | null {
  const trimmed = value.trim();
  if (trimmed === "") {
    return 0;
  }
  if (!/^\d+$/.test(trimmed)) {
    return null;
  }
  const parsed = Number(trimmed);
  if (
    !Number.isSafeInteger(parsed) ||
    parsed < 0 ||
    parsed > MAX_KEEP_RECENT
  ) {
    return null;
  }
  return parsed;
}
