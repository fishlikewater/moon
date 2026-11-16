import { describe, expect, it } from "vitest";
import type { Agent } from "@moon/shared";
import {
  buildDirectChatPickerItems,
  directChatPickerListState,
  filterDirectChatPickerItems,
} from "./directChatPicker";

function agent(overrides: Partial<Agent> = {}): Agent {
  return {
    id: 1,
    name: "小助手",
    persona: "负责回答问题并给出可验收的结论。",
    harness: "zcode",
    customHarnessId: null,
    workspaceDir: null,
    modelConfig: { model: "deepseek-v4.1", reasoningLevel: "max" },
    builtinRole: null,
    createdAt: 1,
    ...overrides,
  };
}

const AGENTS: Agent[] = [
  agent({ id: 1, name: "小助手", persona: "日常陪伴与生活提醒" }),
  agent({
    id: 2,
    name: "Java 架构师",
    persona: "负责后端架构设计与评审",
    harness: "acp",
    modelConfig: { model: "claude-sonnet" },
  }),
  agent({ id: 3, name: "测试专家", persona: "设计测试用例", builtinRole: null }),
];

describe("buildDirectChatPickerItems", () => {
  it("按 id 集合标记已有直聊", () => {
    const items = buildDirectChatPickerItems(AGENTS, new Set([2]));

    expect(items.map((item) => item.existing)).toEqual([false, true, false]);
  });

  it("搜索文本覆盖名字、人设、harness 与模型", () => {
    const items = buildDirectChatPickerItems(AGENTS, new Set());

    expect(items[0]?.searchText).toContain("小助手");
    expect(items[0]?.searchText).toContain("日常陪伴");
    expect(items[0]?.searchText).toContain("zcode");
    expect(items[0]?.searchText).toContain("deepseek-v4.1");
  });
});

describe("filterDirectChatPickerItems", () => {
  const items = buildDirectChatPickerItems(AGENTS, new Set());

  it("空查询返回全部且保持原顺序", () => {
    const filtered = filterDirectChatPickerItems(items, "   ");

    expect(filtered.map((item) => item.agent.id)).toEqual([1, 2, 3]);
  });

  it("名字与人设按大小写不敏感子串匹配", () => {
    expect(
      filterDirectChatPickerItems(items, "java").map((item) => item.agent.id),
    ).toEqual([2]);
    expect(
      filterDirectChatPickerItems(items, "测试用例").map(
        (item) => item.agent.id,
      ),
    ).toEqual([3]);
  });

  it("可按 harness 或模型过滤", () => {
    expect(
      filterDirectChatPickerItems(items, "acp").map((item) => item.agent.id),
    ).toEqual([2]);
    expect(
      filterDirectChatPickerItems(items, "claude").map((item) => item.agent.id),
    ).toEqual([2]);
  });

  it("无匹配返回空且不改动原候选", () => {
    expect(filterDirectChatPickerItems(items, "不存在的智能体")).toEqual([]);
    expect(items).toHaveLength(3);
  });
});

describe("directChatPickerListState", () => {
  it("加载态优先于错误与空列表", () => {
    expect(
      directChatPickerListState({
        loading: true,
        error: "network down",
        resultCount: 0,
      }),
    ).toBe("loading");
  });

  it("加载完成后的列表错误显示错误态，而不是空列表", () => {
    expect(
      directChatPickerListState({
        loading: false,
        error: "network down",
        resultCount: 0,
      }),
    ).toBe("error");
  });

  it("仅在加载成功后无候选时才显示空列表", () => {
    expect(
      directChatPickerListState({ loading: false, error: null, resultCount: 0 }),
    ).toBe("empty");
    expect(
      directChatPickerListState({ loading: false, error: null, resultCount: 2 }),
    ).toBe("results");
  });
});
