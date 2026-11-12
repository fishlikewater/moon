/**
 * 应用外壳导航的纯逻辑：路径分组、导航项与侧栏折叠状态存储。
 *
 * 这些函数不依赖 React，便于直接测试抽屉/折叠在存储异常时的降级行为。
 */

export type ShellSection =
  | "agents"
  | "harnesses"
  | "rooms"
  | "tasks"
  | "sessions"
  | "settings";

export interface ShellNavItem {
  section: ShellSection;
  to: string;
  label: string;
}

export const SHELL_NAV_ITEMS: readonly ShellNavItem[] = [
  { section: "harnesses", to: "/harnesses", label: "Harness" },
  { section: "agents", to: "/agents", label: "智能体" },
  { section: "rooms", to: "/rooms", label: "房间" },
  { section: "tasks", to: "/tasks", label: "任务" },
  { section: "sessions", to: "/sessions", label: "会话" },
  { section: "settings", to: "/settings", label: "设置" },
];

export const SIDEBAR_COLLAPSED_KEY = "moon.sidebar.collapsed";

/** 把当前路径归到唯一导航分组；未知路径按默认首页处理。 */
export function activeSection(pathname: string): ShellSection {
  if (pathname.startsWith("/harnesses")) {
    return "harnesses";
  }
  if (pathname.startsWith("/rooms")) {
    return "rooms";
  }
  if (pathname.startsWith("/tasks")) {
    return "tasks";
  }
  if (pathname.startsWith("/sessions")) {
    return "sessions";
  }
  if (pathname.startsWith("/settings")) {
    return "settings";
  }
  return "agents";
}

export function sectionLabel(section: ShellSection): string {
  return SHELL_NAV_ITEMS.find((item) => item.section === section)?.label ?? "智能体";
}

type SidebarStorage = Pick<Storage, "getItem" | "setItem">;

function browserStorage(): SidebarStorage | null {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

/** 读取折叠状态；无存储、存储不可用或值非法时一律回退为展开。 */
export function readSidebarCollapsed(
  storage: SidebarStorage | null = browserStorage(),
): boolean {
  try {
    return storage?.getItem(SIDEBAR_COLLAPSED_KEY) === "1";
  } catch {
    return false;
  }
}

/** 写入折叠状态；写入失败不影响界面切换。 */
export function writeSidebarCollapsed(
  collapsed: boolean,
  storage: SidebarStorage | null = browserStorage(),
): void {
  try {
    storage?.setItem(SIDEBAR_COLLAPSED_KEY, collapsed ? "1" : "0");
  } catch {
    // 隐私模式或配额异常时放弃持久化，本次会话仍按内存状态切换。
  }
}
