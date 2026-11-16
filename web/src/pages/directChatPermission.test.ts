import { describe, expect, it, vi } from "vitest";
import type { Room } from "@moon/shared";
import type { RoomUpdateInput } from "../api/client";
import { changeDirectPermissionMode } from "./directChatPermission";

function room(overrides: Partial<Room> = {}): Room {
  return {
    id: 7,
    name: "与 小助手 对话",
    roomType: "direct",
    workspaceDir: "E:/tmp/direct-1",
    workspaceDirs: ["E:/tmp/direct-1"],
    moderatorAgentId: null,
    memberIds: [1],
    replyMode: "mentions",
    discussionMaxRounds: 1,
    permissionMode: "auto",
    createdAt: 1,
    lastActivityAt: null,
    ...overrides,
  };
}

describe("changeDirectPermissionMode", () => {
  it("模式未变化时不发请求", async () => {
    const update = vi.fn(async (_id: number, _patch: RoomUpdateInput) =>
      room({ permissionMode: "plan" }),
    );

    await expect(
      changeDirectPermissionMode(room({ permissionMode: "plan" }), "plan", {
        update,
      }),
    ).resolves.toBeNull();
    expect(update).not.toHaveBeenCalled();
  });

  it("切换时只提交 permissionMode 并采用服务端返回的房间", async () => {
    const updated = room({ permissionMode: "plan" });
    const update = vi.fn(async (_id: number, _patch: RoomUpdateInput) => updated);

    await expect(
      changeDirectPermissionMode(room(), "plan", { update }),
    ).resolves.toBe(updated);
    expect(update).toHaveBeenCalledWith(7, { permissionMode: "plan" });
  });

  it("请求失败原样抛出，交由页面展示错误", async () => {
    const failure = new Error("patch failed");
    const update = vi.fn(async (_id: number, _patch: RoomUpdateInput) => {
      throw failure;
    });

    await expect(
      changeDirectPermissionMode(room(), "full", { update }),
    ).rejects.toBe(failure);
  });
});
