import type { AgentBuilderRunInfo } from "@moon/shared";

/** 运行状态标签；当前后端只返回 completed，其余按失败兜底。 */
export function builderRunStatusLabel(status: string): string {
  return status === "completed" ? "已完成" : "失败";
}

/** 耗时：1 秒内显示毫秒，否则保留一位小数秒；缺失或非法返回 null。 */
export function formatBuilderDuration(ms: number | null): string | null {
  if (ms === null || !Number.isFinite(ms) || ms < 0) {
    return null;
  }
  if (ms < 1000) {
    return `${Math.round(ms)} 毫秒`;
  }
  return `${(ms / 1000).toFixed(1)} 秒`;
}

/** token/工具次数千分位；缺失或非法返回 null。 */
export function formatBuilderCount(value: number | null): string | null {
  if (value === null || !Number.isFinite(value)) {
    return null;
  }
  return value.toLocaleString("en-US");
}

/** 摘要行：已完成 · 模型 · 12.3 秒 · 1,234 tokens；缺失字段直接省略。 */
export function builderRunSummary(run: AgentBuilderRunInfo): string {
  const parts = [builderRunStatusLabel(run.status)];
  if (run.model !== null && run.model !== "") {
    parts.push(run.model);
  }
  const duration = formatBuilderDuration(run.durationMs);
  if (duration !== null) {
    parts.push(duration);
  }
  const tokens = formatBuilderCount(run.tokenCount);
  if (tokens !== null) {
    parts.push(`${tokens} tokens`);
  }
  return parts.join(" · ");
}
