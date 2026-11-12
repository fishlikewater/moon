import { describe, expect, it } from "vitest";
import type { AgentBuilderRunInfo } from "@moon/shared";
import {
  builderRunStatusLabel,
  builderRunSummary,
  formatBuilderCount,
  formatBuilderDuration,
} from "./builderRunView";

function run(overrides: Partial<AgentBuilderRunInfo>): AgentBuilderRunInfo {
  return {
    status: "completed",
    model: "zcode / glm-4.6",
    durationMs: 12_300,
    tokenCount: 1_234,
    toolCallCount: 2,
    contextSize: 4_096,
    ...overrides,
  };
}

describe("builderRunStatusLabel", () => {
  it("已完成与失败标签", () => {
    expect(builderRunStatusLabel("completed")).toBe("已完成");
    expect(builderRunStatusLabel("failed")).toBe("失败");
    expect(builderRunStatusLabel("unknown")).toBe("失败");
  });
});

describe("formatBuilderDuration", () => {
  it("缺失与非法值返回 null", () => {
    expect(formatBuilderDuration(null)).toBeNull();
    expect(formatBuilderDuration(Number.NaN)).toBeNull();
    expect(formatBuilderDuration(-1)).toBeNull();
  });

  it("毫秒与秒边界", () => {
    expect(formatBuilderDuration(999)).toBe("999 毫秒");
    expect(formatBuilderDuration(1_000)).toBe("1.0 秒");
    expect(formatBuilderDuration(12_345)).toBe("12.3 秒");
  });
});

describe("formatBuilderCount", () => {
  it("千分位与缺失值", () => {
    expect(formatBuilderCount(1_234)).toBe("1,234");
    expect(formatBuilderCount(0)).toBe("0");
    expect(formatBuilderCount(null)).toBeNull();
  });
});

describe("builderRunSummary", () => {
  it("组合状态、模型、耗时与 token", () => {
    expect(builderRunSummary(run({}))).toBe(
      "已完成 · zcode / glm-4.6 · 12.3 秒 · 1,234 tokens",
    );
  });

  it("缺失模型与 token 时省略对应片段，不显示 null", () => {
    expect(
      builderRunSummary(run({ model: null, tokenCount: null, durationMs: 800 })),
    ).toBe("已完成 · 800 毫秒");
    expect(
      builderRunSummary(run({ model: "", tokenCount: null, durationMs: null })),
    ).toBe("已完成");
  });
});
