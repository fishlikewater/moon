import type { Agent, Room } from "@moon/shared";
import { describe, expect, it } from "vitest";
import { loadNavSummary } from "./useNavSummary";

function agent(id: number): Agent {
  return {
    id,
    name: `智能体 ${id}`,
    persona: "",
    harness: "fake",
    customHarnessId: null,
    workspaceDir: null,
    modelConfig: null,
    builtinRole: null,
    createdAt: id,
  };
}

function room(id: number, createdAt: number): Room {
  return {
    id,
    name: `房间 ${id}`,
    workspaceDir: `E:/tmp/room-${id}`,
    workspaceDirs: [`E:/tmp/room-${id}`],
    moderatorAgentId: null,
    memberIds: [],
    replyMode: "mentions",
    discussionMaxRounds: 5,
    permissionMode: "auto",
    createdAt,
    lastActivityAt: null,
  };
}

describe("loadNavSummary", () => {
  it("局部失败只影响对应区块", async () => {
    const summary = await loadNavSummary({
      listAgents: async () => [agent(1), agent(2), agent(3)],
      listRooms: async () => {
        throw new Error("offline");
      },
    });

    expect(summary.agentCount).toBe(3);
    expect(summary.roomCount).toBeNull();
    expect(summary.recentRooms).toEqual([]);
  });

  it("最近房间按创建时间倒序并截断到 5 个", async () => {
    const rooms = [
      room(1, 30),
      room(2, 10),
      room(3, 70),
      room(4, 20),
      room(5, 60),
      room(6, 40),
      room(7, 50),
    ];

    const summary = await loadNavSummary({
      listAgents: async () => [agent(1)],
      listRooms: async () => rooms,
    });

    expect(summary.roomCount).toBe(7);
    expect(summary.recentRooms.map((item) => item.id)).toEqual([3, 5, 7, 6, 1]);
  });

  it("同步抛出的依赖错误也被隔离", async () => {
    const summary = await loadNavSummary({
      listAgents: () => {
        throw new Error("sync failure");
      },
      listRooms: async () => [room(1, 1)],
    });

    expect(summary.agentCount).toBeNull();
    expect(summary.roomCount).toBe(1);
    expect(summary.recentRooms.map((item) => item.id)).toEqual([1]);
  });
});
