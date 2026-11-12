/** 房间侧栏标签：定义、持久化与键盘导航。 */

export const ROOM_SIDE_TABS = [
  { id: "overview", label: "概览" },
  { id: "files", label: "文件" },
  { id: "members", label: "成员" },
  { id: "queue", label: "队列" },
] as const;

export type RoomSideTab = (typeof ROOM_SIDE_TABS)[number]["id"];

const STORAGE_KEY = "moon.roomSideTab";

/** 只接受已知标签 id；缺失或非法值回退概览。 */
export function parseRoomSideTab(value: string | null | undefined): RoomSideTab {
  const found = ROOM_SIDE_TABS.find((tab) => tab.id === value);
  return found?.id ?? "overview";
}

/** 读取上次标签；存储不可用或值非法时回退概览。 */
export function readRoomSideTab(): RoomSideTab {
  try {
    return parseRoomSideTab(window.localStorage.getItem(STORAGE_KEY));
  } catch {
    return "overview";
  }
}

/** 记住标签；存储不可用时静默放弃，不影响切换。 */
export function persistRoomSideTab(tab: RoomSideTab): void {
  try {
    window.localStorage.setItem(STORAGE_KEY, tab);
  } catch {
    // 隐私模式或存储配额异常：仅失去刷新后的记忆。
  }
}

/** 标签顺序内的键盘导航；左右循环，Home/End 到两端，其他键返回 null。 */
export function nextRoomSideTab(
  current: RoomSideTab,
  key: string,
): RoomSideTab | null {
  const index = ROOM_SIDE_TABS.findIndex((tab) => tab.id === current);
  const count = ROOM_SIDE_TABS.length;
  switch (key) {
    case "ArrowRight":
      return ROOM_SIDE_TABS[(index + 1) % count].id;
    case "ArrowLeft":
      return ROOM_SIDE_TABS[(index - 1 + count) % count].id;
    case "Home":
      return ROOM_SIDE_TABS[0].id;
    case "End":
      return ROOM_SIDE_TABS[count - 1].id;
    default:
      return null;
  }
}
