import { describe, expect, it } from "vitest";
import { formatModeratorTranscript } from "./moderatorProtocol";

const names = new Map<number, string>([
  [1, "前端开发"],
  [2, "代码评审"],
]);

function lookup(agentId: number): string | undefined {
  return names.get(agentId);
}

function fenced(value: unknown): string {
  return ["```json", JSON.stringify(value), "```"].join("\n");
}

describe("formatModeratorTranscript", () => {
  it("折叠分配块并保留成员、执行类型与预期产出", () => {
    const raw = [
      "本轮先实现再复核。",
      fenced({
        type: "assign",
        assignments: [
          {
            agentId: 1,
            instruction: "实现搜索接口与页面",
            kind: "work",
            expectedOutput: "可检索的搜索页面",
          },
          {
            agentId: 2,
            instruction: "复核搜索实现与空态",
            kind: "review",
          },
        ],
      }),
    ].join("\n\n");

    const result = formatModeratorTranscript(raw, lookup);

    expect(result).toContain("本轮先实现再复核。");
    expect(result).toContain("1. [执行] 前端开发：实现搜索接口与页面 → 可检索的搜索页面");
    expect(result).toContain("2. [复核] 代码评审：复核搜索实现与空态");
    expect(result).not.toContain("```json");
    expect(result).not.toContain('"type":"assign"');
  });

  it("折叠验收块并展示验收证据", () => {
    const result = formatModeratorTranscript(
      fenced({
        type: "done",
        summary: "搜索功能已完成",
        verification: ["搜索测试通过", "AC-1 已核对"],
      }),
      lookup,
    );

    expect(result).toContain("主持人验收：搜索功能已完成");
    expect(result).toContain("1. 搜索测试通过");
    expect(result).toContain("2. AC-1 已核对");
    expect(result).not.toContain('"verification"');
  });

  it("普通文本与非协议代码块原样返回", () => {
    const text = "普通回复\n\n```ts\nconst value = 1;\n```";
    expect(formatModeratorTranscript(text, lookup)).toBe(text);
  });

  it("非法 JSON 不触发折叠", () => {
    const text = "说明\n\n```json\n{not-json}\n```";
    expect(formatModeratorTranscript(text, lookup)).toBe(text);
  });

  it("非主持人输出即使形似协议块也保持原样", () => {
    const text = fenced({
      type: "assign",
      assignments: [{ agentId: 1, instruction: "普通成员的计划" }],
    });
    expect(formatModeratorTranscript(text, lookup, false)).toBe(text);
  });

  it("多个协议块只保留最后一个摘要并剥离全部块", () => {
    const raw = [
      fenced({ type: "assign", assignments: [{ agentId: 1, instruction: "旧计划" }] }),
      fenced({ type: "done", summary: "最终验收通过", verification: ["已复核"] }),
    ].join("\n");

    const result = formatModeratorTranscript(raw, lookup);

    expect(result).toContain("主持人验收：最终验收通过");
    expect(result).not.toContain("旧计划");
    expect(result).not.toContain("```");
  });
});
