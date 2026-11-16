import type { Agent } from "@moon/shared";

export interface DirectChatPickerItem {
  agent: Agent;
  /** 该智能体是否已有直聊房间（选择后复用原房间）。 */
  existing: boolean;
  searchText: string;
}

/** 候选列表：预计算搜索文本，已有直聊按 agent id 标记。 */
export function buildDirectChatPickerItems(
  agents: readonly Agent[],
  existingAgentIds: ReadonlySet<number>,
): DirectChatPickerItem[] {
  return agents.map((agent) => ({
    agent,
    existing: existingAgentIds.has(agent.id),
    searchText: [
      agent.name,
      agent.persona,
      agent.harness,
      agent.modelConfig?.model ?? "",
      agent.modelConfig?.reasoningLevel ?? "",
    ]
      .join(" ")
      .toLowerCase(),
  }));
}

/** 大小写不敏感的子串过滤；空查询保持原顺序返回全部候选。 */
export function filterDirectChatPickerItems(
  items: readonly DirectChatPickerItem[],
  query: string,
): DirectChatPickerItem[] {
  const keyword = query.trim().toLowerCase();
  if (keyword === "") {
    return [...items];
  }
  return items.filter((item) => item.searchText.includes(keyword));
}

export type DirectChatPickerListState =
  | "loading"
  | "error"
  | "empty"
  | "results";

/**
 * 选择器列表区的展示状态。加载态优先于错误与空列表，
 * 避免候选未就绪时误报「还没有智能体」。
 */
export function directChatPickerListState(input: {
  loading: boolean;
  error: string | null;
  resultCount: number;
}): DirectChatPickerListState {
  if (input.loading) {
    return "loading";
  }
  if (input.error !== null) {
    return "error";
  }
  return input.resultCount === 0 ? "empty" : "results";
}
