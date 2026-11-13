import type { AgentRun, ContextPreview } from "@moon/shared";

/** 展示用 token 估算：固定按 4 字符 ≈ 1 token，向上取整，不宣称精确。 */
export function estimateTokens(chars: number): number {
  if (!Number.isFinite(chars) || chars <= 0) {
    return 0;
  }
  return Math.ceil(chars / 4);
}

/** 上下文使用百分比；非法或缺失返回 null，结果钳制在 0–100。 */
export function usagePercent(
  used: number | null | undefined,
  size: number | null | undefined,
): number | null {
  if (
    typeof used !== "number" ||
    typeof size !== "number" ||
    !Number.isFinite(used) ||
    !Number.isFinite(size) ||
    size <= 0
  ) {
    return null;
  }
  const percent = Math.min(100, Math.max(0, (used / size) * 100));
  return Math.round(percent * 10) / 10;
}

export function formatContextMetric(preview: ContextPreview): string {
  return `约 ${preview.estimatedTokens} tokens · ${preview.effectiveChars} 字`;
}

export function formatContextSummary(preview: ContextPreview): string {
  if (preview.sessionReused) {
    const parts = ["复用会话"];
    if (preview.includedMessages > 0) {
      parts.push(`纳入 ${preview.includedMessages} 条`);
    }
    if (preview.compressedMessages > 0) {
      parts.push(`压缩 ${preview.compressedMessages} 条`);
    }
    if (preview.omittedMessages > 0) {
      parts.push(`省略 ${preview.omittedMessages} 条`);
    }
    if (preview.includedMessages === 0 && preview.omittedMessages === 0) {
      parts.push("仅发送当前指令");
    }
    return parts.join(" · ");
  }
  if (!preview.enabled) {
    return `压缩已关闭 · 纳入 ${preview.includedMessages} 条`;
  }
  const parts = [`纳入 ${preview.includedMessages} 条`];
  if (preview.compressedMessages > 0) {
    parts.push(`压缩 ${preview.compressedMessages} 条`);
  }
  if (preview.omittedMessages > 0) {
    parts.push(`省略 ${preview.omittedMessages} 条`);
  }
  return parts.join(" · ");
}

/** 运行详情的两行用量文案；缺少对应字段时返回 null，由页面决定不渲染。 */
export function formatUsage(
  run: Pick<AgentRun, "tokenCount" | "contextSize">,
): { tokens: string | null; context: string | null } {
  const tokens =
    typeof run.tokenCount === "number" ? `tokens ${run.tokenCount}` : null;
  const percent = usagePercent(run.tokenCount ?? null, run.contextSize ?? null);
  const context =
    typeof run.contextSize === "number"
      ? `上下文 ${run.tokenCount ?? "?"} / ${run.contextSize}${
          percent === null ? "" : `（${percent}%）`
        }`
      : null;
  return { tokens, context };
}
