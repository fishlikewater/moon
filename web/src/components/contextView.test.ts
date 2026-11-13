import { describe, expect, it } from "vitest";
import type { ContextPreview } from "@moon/shared";
import {
  estimateTokens,
  formatContextMetric,
  formatContextSummary,
  formatUsage,
  usagePercent,
} from "./contextView";

function preview(overrides: Partial<ContextPreview> = {}): ContextPreview {
  return {
    enabled: true,
    budgetChars: 24_000,
    rawChars: 30_000,
    effectiveChars: 12_000,
    estimatedTokens: 3_000,
    includedMessages: 20,
    compressedMessages: 14,
    omittedMessages: 0,
    sessionReused: false,
    ...overrides,
  };
}

describe("estimateTokens", () => {
  it("按 4 字符向上取整", () => {
    expect(estimateTokens(0)).toBe(0);
    expect(estimateTokens(1)).toBe(1);
    expect(estimateTokens(4)).toBe(1);
    expect(estimateTokens(5)).toBe(2);
  });

  it("非法值退化为 0", () => {
    expect(estimateTokens(-3)).toBe(0);
    expect(estimateTokens(Number.NaN)).toBe(0);
  });
});

describe("usagePercent", () => {
  it("计算并保留一位小数", () => {
    expect(usagePercent(0, 1000)).toBe(0);
    expect(usagePercent(42, 1000)).toBe(4.2);
    expect(usagePercent(1000, 1000)).toBe(100);
  });

  it("钳制超界并拒绝缺失值", () => {
    expect(usagePercent(2000, 1000)).toBe(100);
    expect(usagePercent(-5, 1000)).toBe(0);
    expect(usagePercent(null, 1000)).toBeNull();
    expect(usagePercent(10, null)).toBeNull();
    expect(usagePercent(10, 0)).toBeNull();
  });
});

describe("formatContextMetric / formatContextSummary", () => {
  it("展示估算 token 与字符数", () => {
    expect(formatContextMetric(preview())).toBe("约 3000 tokens · 12000 字");
  });

  it("启用时展示纳入、压缩与省略条数", () => {
    expect(formatContextSummary(preview({ omittedMessages: 3 }))).toBe(
      "纳入 20 条 · 压缩 14 条 · 省略 3 条",
    );
    expect(formatContextSummary(preview({ compressedMessages: 0 }))).toBe(
      "纳入 20 条",
    );
  });

  it("禁用与复用会话使用独立文案", () => {
    expect(formatContextSummary(preview({ enabled: false }))).toBe(
      "压缩已关闭 · 纳入 20 条",
    );
    expect(formatContextSummary(preview({ sessionReused: true }))).toBe(
      "复用会话 · 纳入 20 条 · 压缩 14 条",
    );
    expect(
      formatContextSummary(
        preview({
          sessionReused: true,
          includedMessages: 0,
          compressedMessages: 0,
        }),
      ),
    ).toBe("复用会话 · 仅发送当前指令");
  });
});

describe("formatUsage", () => {
  it("有 token 与上下文大小时给出两行文案", () => {
    expect(formatUsage({ tokenCount: 42, contextSize: 1000 })).toEqual({
      tokens: "tokens 42",
      context: "上下文 42 / 1000（4.2%）",
    });
  });

  it("缺失字段不生成文案", () => {
    expect(formatUsage({})).toEqual({ tokens: null, context: null });
    expect(formatUsage({ tokenCount: 42, contextSize: null })).toEqual({
      tokens: "tokens 42",
      context: null,
    });
    expect(formatUsage({ tokenCount: null, contextSize: 1000 })).toEqual({
      tokens: null,
      context: "上下文 ? / 1000",
    });
  });
});
