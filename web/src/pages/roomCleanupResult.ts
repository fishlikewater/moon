import type { MessageCleanupResult } from "@moon/shared";

/**
 * 清理结果提示。
 *
 * `deleted` 是物理删除数；`hidden` 是因讨论/任务/委派记录保留而不再出现在
 * 聊天窗口的数量。两者都为零时才是真正的「无可清理」。
 */
export function formatCleanupResult(result: MessageCleanupResult): string {
  const parts: string[] = [];
  if (result.deleted > 0) {
    parts.push(`已清理 ${result.deleted} 条聊天记录`);
  }
  if (result.hidden > 0) {
    parts.push(
      `已从聊天窗口隐藏 ${result.hidden} 条讨论/任务/委派记录（记录本身保留）`,
    );
  }
  if (parts.length === 0) {
    return "没有可清理的聊天记录";
  }
  let text = parts.join("，");
  if (result.artifactsDeleted > 0) {
    text += `，同时删除 ${result.artifactsDeleted} 个归档文件`;
  }
  if (result.attachmentsDeleted > 0) {
    text += `、${result.attachmentsDeleted} 个附件`;
  }
  return text;
}
