import type { ReactNode } from "react";
import { Link, NavLink } from "react-router-dom";
import type { NavSummary } from "../../useNavSummary";
import {
  SHELL_NAV_ITEMS,
  type ShellSection,
} from "./shellNav";

interface AppSidebarProps {
  brand: ReactNode;
  summary: NavSummary;
  collapsed?: boolean;
  variant?: "desktop" | "drawer";
  onToggleCollapsed?: () => void;
  onNavigate?: () => void;
  onClose?: () => void;
}

function AgentsIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      <path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2" />
      <circle cx="9" cy="7" r="4" />
      <path d="M22 21v-2a4 4 0 0 0-3-3.87" />
      <path d="M16 3.13a4 4 0 0 1 0 7.75" />
    </svg>
  );
}

function RoomsIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      <path d="M21 15a2 2 0 0 1-2 2H8l-4 3.5V5a2 2 0 0 1 2-2h13a2 2 0 0 1 2 2Z" />
      <path d="M8 9h8" />
      <path d="M8 13h5" />
    </svg>
  );
}

function TasksIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      <rect x="5" y="4" width="14" height="17" rx="2" />
      <path d="M9 4a3 3 0 0 1 6 0" />
      <path d="M9 11h6" />
      <path d="M9 15h4" />
    </svg>
  );
}

function SessionsIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      <rect x="3" y="4" width="18" height="16" rx="2" />
      <path d="m7 9 3 3-3 3" />
      <path d="M13 15h4" />
    </svg>
  );
}

function HarnessesIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      <path d="M9 2v6" />
      <path d="M15 2v6" />
      <path d="M6 8h12v3a6 6 0 0 1-12 0V8Z" />
      <path d="M12 17v5" />
    </svg>
  );
}

function SectionIcon({ section }: { section: ShellSection }) {
  if (section === "harnesses") {
    return <HarnessesIcon />;
  }
  if (section === "rooms") {
    return <RoomsIcon />;
  }
  if (section === "tasks") {
    return <TasksIcon />;
  }
  if (section === "sessions") {
    return <SessionsIcon />;
  }
  if (section === "settings") {
    return <SettingsIcon />;
  }
  return <AgentsIcon />;
}

function SettingsIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 15a1.7 1.7 0 0 0 .34 1.87l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06A1.7 1.7 0 0 0 15 19.4a1.7 1.7 0 0 0-1 .6 1.7 1.7 0 0 0-.4 1.1V21a2 2 0 1 1-4 0v-.09A1.7 1.7 0 0 0 8.6 19.4a1.7 1.7 0 0 0-1.87.34l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06A1.7 1.7 0 0 0 4.6 15a1.7 1.7 0 0 0-.6-1 1.7 1.7 0 0 0-1.1-.4H3a2 2 0 1 1 0-4h.09A1.7 1.7 0 0 0 4.6 8.6a1.7 1.7 0 0 0-.34-1.87l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.7 1.7 0 0 0 9 4.6a1.7 1.7 0 0 0 1-.6 1.7 1.7 0 0 0 .4-1.1V3a2 2 0 1 1 4 0v.09A1.7 1.7 0 0 0 15.4 4.6a1.7 1.7 0 0 0 1.87-.34l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06A1.7 1.7 0 0 0 19.4 9c.14.36.35.7.6 1 .3.27.68.41 1.1.4H21a2 2 0 1 1 0 4h-.09a1.7 1.7 0 0 0-1.51.6Z" />
    </svg>
  );
}

function CollapseIcon({ collapsed }: { collapsed: boolean }) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      {collapsed ? <path d="M9 6l6 6-6 6" /> : <path d="M15 6l-6 6 6 6" />}
    </svg>
  );
}

function CloseIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      <path d="M6 6l12 12" />
      <path d="M18 6L6 18" />
    </svg>
  );
}

function countFor(section: ShellSection, summary: NavSummary): number | null {
  if (section === "agents") {
    return summary.agentCount;
  }
  if (section === "rooms") {
    return summary.roomCount;
  }
  return null;
}

/**
 * 左侧导航栏：品牌、主导航、最近房间与折叠/关闭入口。
 *
 * 抽屉模式只改变容器类名与关闭入口，导航内容与桌面侧栏共用同一份结构。
 */
export function AppSidebar({
  brand,
  summary,
  collapsed = false,
  variant = "desktop",
  onToggleCollapsed,
  onNavigate,
  onClose,
}: AppSidebarProps) {
  const drawer = variant === "drawer";
  const recentEmpty =
    summary.roomCount === 0
      ? "还没有房间"
      : summary.roomCount === null
        ? "房间数据不可用"
        : "暂无最近房间";

  return (
    <aside
      id="app-sidebar"
      className={`app-sidebar${collapsed && !drawer ? " is-collapsed" : ""}${
        drawer ? " is-drawer" : ""
      }`}
      aria-label={drawer ? "导航抽屉" : "主导航侧栏"}
    >
      <div className="app-sidebar-brand" data-tauri-drag-region="deep">
        <div className="app-sidebar-brand-slot">{brand}</div>
        {drawer ? (
          <button
            type="button"
            className="app-sidebar-icon-button"
            aria-label="关闭导航"
            title="关闭导航"
            onClick={onClose}
          >
            <CloseIcon />
          </button>
        ) : (
          <button
            type="button"
            className="app-sidebar-icon-button"
            aria-label={collapsed ? "展开侧栏" : "折叠侧栏"}
            aria-pressed={collapsed}
            title={collapsed ? "展开侧栏" : "折叠侧栏"}
            onClick={onToggleCollapsed}
          >
            <CollapseIcon collapsed={collapsed} />
          </button>
        )}
      </div>

      <nav className="app-sidebar-nav" aria-label="主导航">
        {SHELL_NAV_ITEMS.map((item) => {
          const count = countFor(item.section, summary);
          const showsCount =
            item.section === "agents" || item.section === "rooms";
          return (
            <NavLink
              key={item.section}
              to={item.to}
              onClick={onNavigate}
              title={collapsed && !drawer ? item.label : undefined}
              className={({ isActive }) =>
                isActive ? "app-nav-link active" : "app-nav-link"
              }
            >
              <SectionIcon section={item.section} />
              <span className="app-nav-label">{item.label}</span>
              {showsCount && (
                <span
                  className="app-nav-count"
                  aria-label={count === null ? "数量不可用" : `${count} 项`}
                >
                  {count === null ? "—" : count}
                </span>
              )}
            </NavLink>
          );
        })}
      </nav>

      {!collapsed && (
        <div className="app-sidebar-recent">
          <p className="app-sidebar-section-title">最近房间</p>
          {summary.recentRooms.length === 0 ? (
            <p className="app-sidebar-empty">{recentEmpty}</p>
          ) : (
            <ul className="app-sidebar-recent-list">
              {summary.recentRooms.map((room) => (
                <li key={room.id}>
                  <Link
                    to={`/rooms/${room.id}`}
                    className="app-sidebar-recent-link"
                    onClick={onNavigate}
                    title={room.workspaceDir}
                  >
                    <span className="app-sidebar-recent-name">{room.name}</span>
                    <span className="app-sidebar-recent-meta">
                      {room.memberIds.length} 人
                    </span>
                  </Link>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}

      <div className="app-sidebar-footer">
        <span className="app-sidebar-status-dot" aria-hidden="true" />
        {!collapsed && <span>本地 · 单机</span>}
      </div>
    </aside>
  );
}
