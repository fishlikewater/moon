import { describe, expect, it, vi } from "vitest";
import type { Agent, Room } from "@moon/shared";
import { ApiError } from "../api/client";
import {
  createDirectChatEntry,
  resolveDirectChatEntry,
  shouldCreateDirectChat,
  shouldRedirectToDirectChat,
  type DirectChatEntryPhase,
} from "./directChatEntry";

const PHASES: readonly DirectChatEntryPhase[] = [
  "loading",
  "navigate",
  "offer",
  "error",
];

const AGENT: Agent = {
  id: 1,
  name: "小助手",
  persona: "回答问题的智能体。",
  harness: "fake",
  customHarnessId: null,
  workspaceDir: null,
  modelConfig: null,
  builtinRole: null,
  createdAt: 1,
};

function directRoom(id: number, agentId: number): Room {
  return {
    id,
    name: `与 小助手 对话`,
    roomType: "direct",
    workspaceDir: `E:/tmp/direct-${agentId}`,
    workspaceDirs: [`E:/tmp/direct-${agentId}`],
    moderatorAgentId: null,
    memberIds: [agentId],
    replyMode: "mentions",
    discussionMaxRounds: 1,
    permissionMode: "auto",
    createdAt: 1,
    lastActivityAt: null,
  };
}

describe("shouldCreateDirectChat", () => {
  it("只有引导态且未在创建时才允许写库", () => {
    expect(shouldCreateDirectChat({ phase: "offer", creating: false })).toBe(
      true,
    );
    expect(shouldCreateDirectChat({ phase: "offer", creating: true })).toBe(
      false,
    );
  });

  it("查询中/跳转中/错误态一律不触发创建", () => {
    for (const phase of PHASES) {
      if (phase === "offer") {
        continue;
      }
      expect(shouldCreateDirectChat({ phase, creating: false })).toBe(false);
    }
  });
});

describe("shouldRedirectToDirectChat", () => {
  it("仅命中已有直聊时跳转", () => {
    expect(shouldRedirectToDirectChat("navigate")).toBe(true);
    for (const phase of PHASES) {
      if (phase === "navigate") {
        continue;
      }
      expect(shouldRedirectToDirectChat(phase)).toBe(false);
    }
  });
});

describe("resolveDirectChatEntry", () => {
  it("未创建直聊时停在引导态（只读查询，创建依赖不在此函数内）", async () => {
    const result = await resolveDirectChatEntry(1, {
      listAgents: async () => [AGENT],
      getDirectChat: async () => {
        throw new ApiError(404, "NOT_FOUND", "该智能体还没有直聊");
      },
    });

    expect(result).toMatchObject({ phase: "offer", agent: AGENT });
  });

  it("已有直聊返回跳转房间 id", async () => {
    const result = await resolveDirectChatEntry(1, {
      listAgents: async () => [AGENT],
      getDirectChat: async () => directRoom(42, 1),
    });

    expect(result).toMatchObject({ phase: "navigate", roomId: 42 });
  });

  it("智能体缺失或地址非法时返回错误", async () => {
    const missing = await resolveDirectChatEntry(2, {
      listAgents: async () => [AGENT],
      getDirectChat: async () => directRoom(42, 1),
    });
    expect(missing.phase).toBe("error");

    const invalid = await resolveDirectChatEntry(null, {
      listAgents: async () => [AGENT],
      getDirectChat: async () => directRoom(42, 1),
    });
    expect(invalid.phase).toBe("error");
  });

  it("非 404 查询失败上报错误，不降级成引导态", async () => {
    const result = await resolveDirectChatEntry(1, {
      listAgents: async () => [AGENT],
      getDirectChat: async () => {
        throw new ApiError(500, "INTERNAL_ERROR", "服务异常");
      },
    });

    expect(result.phase).toBe("error");
    if (result.phase === "error") {
      expect(result.message).toBe("服务异常");
    }
  });
});

describe("createDirectChatEntry", () => {
  it("显式调用才写库，并返回幂等接口的房间 id", async () => {
    const create = vi.fn(async (agentId: number) => directRoom(43, agentId));

    await expect(createDirectChatEntry(1, { create })).resolves.toBe(43);
    expect(create).toHaveBeenCalledTimes(1);
    expect(create).toHaveBeenCalledWith(1);
  });
});
