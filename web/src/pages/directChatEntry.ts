/** 直聊跳板页的纯逻辑：只读查询、跳转与创建时机。 */

import type { Agent, Room } from "@moon/shared";
import { ApiError, errorMessage } from "../api/client";

/**
 * 跳板页阶段：
 *
 * - `loading`：正在只读查询智能体与直聊；
 * - `navigate`：已存在直聊，由 effect 跳转到直聊视图；
 * - `offer`：智能体存在但还没有直聊，等待用户点击「发起对话」；
 * - `error`：地址非法、智能体不存在或请求失败。
 */
export type DirectChatEntryPhase = "loading" | "navigate" | "offer" | "error";

export interface DirectChatEntryState {
  phase: DirectChatEntryPhase;
  creating: boolean;
}

/**
 * 是否允许发起创建。
 *
 * 渲染路径只读：只有用户点击按钮（`offer` 阶段）且没有正在创建时才写库，
 * 刷新或直达 `/chat/:agentId` 不会隐式创建直聊。
 */
export function shouldCreateDirectChat(state: DirectChatEntryState): boolean {
  return state.phase === "offer" && !state.creating;
}

/** 已命中直聊时跳转；跳转本身由 effect 执行，不在渲染期产生副作用。 */
export function shouldRedirectToDirectChat(
  phase: DirectChatEntryPhase,
): boolean {
  return phase === "navigate";
}

export interface DirectChatLookupDeps {
  listAgents: () => Promise<Agent[]>;
  getDirectChat: (agentId: number) => Promise<Room>;
}

export type DirectChatLookupResult =
  | { phase: "navigate"; agent: Agent; roomId: number }
  | { phase: "offer"; agent: Agent }
  | { phase: "error"; message: string };

/**
 * 跳板页的只读解析：查智能体 → 查已有直聊。
 *
 * 这里只调用 GET（`getDirectChat`），创建必须由「发起对话」按钮走
 * `createDirectChatEntry`；测试以此断言渲染路径零写操作。
 */
export async function resolveDirectChatEntry(
  agentId: number | null,
  deps: DirectChatLookupDeps,
): Promise<DirectChatLookupResult> {
  if (agentId === null) {
    return { phase: "error", message: "非法的智能体地址" };
  }

  let agents: Agent[];
  try {
    agents = await deps.listAgents();
  } catch (cause) {
    return { phase: "error", message: errorMessage(cause) };
  }
  const agent = agents.find((candidate) => candidate.id === agentId) ?? null;
  if (agent === null) {
    return { phase: "error", message: "智能体不存在或已被删除" };
  }

  try {
    const room = await deps.getDirectChat(agentId);
    return { phase: "navigate", agent, roomId: room.id };
  } catch (cause) {
    if (cause instanceof ApiError && cause.status === 404) {
      // 还没有直聊：停在引导卡，等待用户显式发起。
      return { phase: "offer", agent };
    }
    return { phase: "error", message: errorMessage(cause) };
  }
}

export interface DirectChatCreateDeps {
  create: (agentId: number) => Promise<Room>;
}

/** 按钮回调的唯一写入口；幂等接口，已有直聊时返回原房间 id。 */
export async function createDirectChatEntry(
  agentId: number,
  deps: DirectChatCreateDeps,
): Promise<number> {
  const room = await deps.create(agentId);
  return room.id;
}
