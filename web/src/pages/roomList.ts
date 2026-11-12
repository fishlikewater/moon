/**
 * 房间列表的过滤、排序与展示纯函数。
 *
 * 排序默认按「最近活跃」：最近一条消息时间，无消息回退创建时间；
 * 名称排序用中文语序（拼音）。相对时间接受注入的 now，便于测试。
 */
import type { Room } from "@moon/shared";

export type RoomSort = "activity" | "name";

const MINUTE = 60_000;
const HOUR = 3_600_000;
const DAY = 86_400_000;

/** 名称或任一工作目录包含查询串（大小写不敏感）；空查询返回全部。 */
export function filterRooms(rooms: Room[], query: string): Room[] {
  const needle = query.trim().toLowerCase();
  if (needle === "") {
    return rooms;
  }
  return rooms.filter(
    (room) =>
      room.name.toLowerCase().includes(needle) ||
      room.workspaceDirs.some((dir) => dir.toLowerCase().includes(needle)),
  );
}

/** 不修改入参；时间相同用 id 兜底保证顺序稳定。 */
export function sortRooms(rooms: Room[], sort: RoomSort): Room[] {
  const copy = [...rooms];
  if (sort === "name") {
    copy.sort(
      (left, right) =>
        left.name.localeCompare(right.name, "zh-Hans-CN") ||
        left.id - right.id,
    );
    return copy;
  }
  copy.sort((left, right) => {
    const leftTime = left.lastActivityAt ?? left.createdAt;
    const rightTime = right.lastActivityAt ?? right.createdAt;
    return rightTime - leftTime || left.id - right.id;
  });
  return copy;
}

export function visibleRooms(
  rooms: Room[],
  query: string,
  sort: RoomSort,
): Room[] {
  return sortRooms(filterRooms(rooms, query), sort);
}

/** 最近活动相对时间；超过 7 天显示本地日期。 */
export function formatActivity(ms: number | null, nowMs: number): string {
  if (ms === null) {
    return "暂无活动";
  }
  const diff = Math.max(0, nowMs - ms);
  if (diff < MINUTE) {
    return "刚刚";
  }
  if (diff < HOUR) {
    return `${Math.floor(diff / MINUTE)} 分钟前`;
  }
  if (diff < DAY) {
    return `${Math.floor(diff / HOUR)} 小时前`;
  }
  if (diff < 7 * DAY) {
    return `${Math.floor(diff / DAY)} 天前`;
  }
  const date = new Date(ms);
  const pad = (value: number) => String(value).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

export interface AvatarStack {
  /** 最多 max 个成员 id。 */
  shown: number[];
  /** 被折叠的成员数。 */
  overflow: number;
}

export function avatarStack(ids: number[], max = 4): AvatarStack {
  return {
    shown: ids.slice(0, max),
    overflow: Math.max(0, ids.length - max),
  };
}
