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
  /** 预计从聊天窗口隐藏的受保护消息数（不物理删除） */
  hidden: number;
}

/**
 * 计算清理预览。
 *
 * 服务端按消息 id 推进聊天窗口水位：keep=0 隐藏当前全部可见消息，
 * 否则隐藏保留窗口起点之前的消息。这里用同一规则做上限估计；
 * 前端不知道后端 protected_ids（被委派/讨论引用的纯聊天），
 * 因此 deleting 可能略高于实际值，最终以接口返回为准。
 */
export function cleanupPreview(
  messages: readonly Message[],
  keepRecent: number,
): CleanupPreview {
  const eligible = messages.filter(isPureChatMessage);
  const keep = Math.min(Math.max(keepRecent, 0), eligible.length);
  const floor =
    keep === 0
      ? messages.reduce((max, message) => Math.max(max, message.id), 0)
      : eligible[eligible.length - keep].id - 1;
  const inRange = messages.filter((message) => message.id <= floor);
  const deleting = inRange.filter(isPureChatMessage).length;
  return {
    eligible: eligible.length,
    deleting,
    keeping: keep,
    hidden: inRange.length - deleting,
  };
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
