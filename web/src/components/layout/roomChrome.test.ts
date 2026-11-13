import type { Room } from "@moon/shared";
import { describe, expect, it } from "vitest";
import { buildRoomChrome, resolveTopbarTitle } from "./roomChrome";

function makeRoom(overrides: Partial<Room> = {}): Room {
  return {
    id: 13,
    name: "团购券V2版本",
    workspaceDir: "E:\\Projects\\moon",
    workspaceDirs: ["E:\\Projects\\moon"],
    moderatorAgentId: null,
    memberIds: [],
    replyMode: "mentions",
    discussionMaxRounds: 5,
    permissionMode: "auto",
    createdAt: 1_700_000_000_000,
    lastActivityAt: null,
    ...overrides,
  };
}

const roomChrome = {
  title: "团购券V2版本",
  backTo: "/rooms",
  backLabel: "返回房间列表",
  meta: "E:\\Projects\\moon",
};

describe("resolveTopbarTitle", () => {
  it("没有房间上下文时使用导航分区名", () => {
    expect(resolveTopbarTitle(null, "房间")).toBe("房间");
  });

  it("房间页优先显示房间名", () => {
    expect(resolveTopbarTitle(roomChrome, "房间")).toBe("团购券V2版本");
  });
});

describe("buildRoomChrome", () => {
  it("非法房间 id 不注入顶栏信息", () => {
    expect(buildRoomChrome(makeRoom(), null)).toBeNull();
    expect(buildRoomChrome(null, null)).toBeNull();
  });

  it("房间未加载时仍给出可读标题与返回入口", () => {
    expect(buildRoomChrome(null, 13)).toEqual({
      title: "房间 #13",
      backTo: "/rooms",
      backLabel: "返回房间列表",
      meta: undefined,
    });
  });

  it("单目录房间显示目录路径", () => {
    expect(buildRoomChrome(makeRoom(), 13)).toEqual(roomChrome);
  });

  it("多目录房间追加目录数量摘要", () => {
    expect(
      buildRoomChrome(
        makeRoom({
          workspaceDirs: ["E:\\Projects\\moon", "D:\\work\\other"],
        }),
        13,
      )?.meta,
    ).toBe("E:\\Projects\\moon · 另有 1 个目录");  });
});
