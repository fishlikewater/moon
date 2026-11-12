import { describe, expect, it } from "vitest";
import type { Room } from "@moon/shared";
import {
  avatarStack,
  filterRooms,
  formatActivity,
  sortRooms,
} from "./roomList";

function room(id: number, overrides: Partial<Room> = {}): Room {
  return {
    id,
    name: `房间 ${id}`,
    workspaceDir: "C:/ws",
    workspaceDirs: ["C:/ws"],
    moderatorAgentId: null,
    memberIds: [],
    replyMode: "moderator",
    discussionMaxRounds: 5,
    permissionMode: "auto",
    createdAt: 1000,
    lastActivityAt: null,
    ...overrides,
  };
}

describe("filterRooms", () => {
  it("空查询返回全部房间（原顺序）", () => {
    const rooms = [room(1), room(2)];
    expect(filterRooms(rooms, "  ")).toHaveLength(2);
  });

  it("按名称与目录路径过滤且大小写不敏感", () => {
    const rooms = [
      room(1, { name: "前端重构" }),
      room(2, { name: "后端", workspaceDirs: ["D:/Proj/Migration"] }),
      room(3, { name: "文档" }),
    ];
    expect(filterRooms(rooms, "前端").map((item) => item.id)).toEqual([1]);
    expect(filterRooms(rooms, "migration").map((item) => item.id)).toEqual([2]);
    expect(filterRooms(rooms, "不存在")).toEqual([]);
  });
});

describe("sortRooms", () => {
  it("最近活跃优先：lastActivityAt 缺失回退 createdAt", () => {
    // createdAt 与 id 顺序刻意相反：若丢失回退（如改成 ?? 0），id 兜底会把 1 排到 3 前。
    const rooms = [
      room(1, { createdAt: 500 }),
      room(2, { createdAt: 100, lastActivityAt: 900 }),
      room(3, { createdAt: 600 }),
    ];
    expect(sortRooms(rooms, "activity").map((item) => item.id)).toEqual([
      2, 3, 1,
    ]);
  });

  it("活动时间相同按 id 升序兜底", () => {
    const rooms = [
      room(4, { createdAt: 1, lastActivityAt: 900 }),
      room(2, { createdAt: 1, lastActivityAt: 900 }),
    ];
    expect(sortRooms(rooms, "activity").map((item) => item.id)).toEqual([2, 4]);
  });

  it("名称排序按中文语序且不改动原数组", () => {
    const rooms = [
      room(1, { name: "波房" }),
      room(2, { name: "阿房" }),
    ];
    const sorted = sortRooms(rooms, "name");
    expect(sorted.map((item) => item.name)).toEqual(["阿房", "波房"]);
    expect(rooms.map((item) => item.name)).toEqual(["波房", "阿房"]);
  });
});

describe("formatActivity", () => {
  const now = new Date(2026, 1, 1, 12, 0, 0).getTime();

  it("无活动显示占位文案", () => {
    expect(formatActivity(null, now)).toBe("暂无活动");
  });

  it("分钟/小时/天/日期边界", () => {
    expect(formatActivity(now - 30_000, now)).toBe("刚刚");
    expect(formatActivity(now - 5 * 60_000, now)).toBe("5 分钟前");
    expect(formatActivity(now - 3 * 3_600_000, now)).toBe("3 小时前");
    expect(formatActivity(now - 2 * 86_400_000, now)).toBe("2 天前");
    const old = new Date(2026, 0, 5).getTime();
    expect(formatActivity(old, now)).toBe("2026-01-05");
  });
});

describe("avatarStack", () => {
  it("不超过上限时全部展示", () => {
    expect(avatarStack([1, 2])).toEqual({ shown: [1, 2], overflow: 0 });
  });

  it("超过上限折叠多余成员并计数", () => {
    expect(avatarStack([1, 2, 3, 4, 5, 6])).toEqual({
      shown: [1, 2, 3, 4],
      overflow: 2,
    });
  });
});
