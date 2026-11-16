import { useEffect, useState, type ReactNode } from "react";
import { Link, NavLink, useLocation, useNavigate } from "react-router-dom";
import type { Agent, Room } from "@moon/shared";
import { api, errorMessage } from "../../api/client";
import { AgentAvatar } from "../AgentAvatar";
import type { NavSummary } from "../../useNavSummary";
import {
  readSidebarDirectsCollapsed,
  SHELL_NAV_ITEMS,
  writeSidebarDirectsCollapsed,
  type ShellSection,
} from "./shellNav";

/** 侧栏直聊条目：一条直聊对应唯一智能体。 */
export interface DirectChatEntry {
  roomId: number;
  agentId: number;
  agentName: string;
  lastActivityAt: number | null;
}

interface AppSidebarProps {
  brand: ReactNode;
  summary: NavSummary;
  collapsed?: boolean;
  variant?: "desktop" | "drawer";
  onToggleCollapsed?: () => void;
  onNavigate?: () => void;
  onClose?: () => void;
  /** 直聊条目；不传时组件自行拉取（测试可注入以跳过网络）。 */
  directChats?: readonly DirectChatEntry[];
}

/** 直聊房间 + 智能体列表 → 侧栏条目；成员缺失的直聊不展示。 */
export function buildDirectChatEntries(
  rooms: readonly Room[],
  agents: readonly Agent[],
): DirectChatEntry[] {
  const byId = new Map(agents.map((agent) => [agent.id, agent]));
  const entries: DirectChatEntry[] = [];
  for (const room of rooms) {
    const agentId = room.memberIds[0];
    const agent = agentId === undefined ? undefined : byId.get(agentId);
    if (agentId === undefined || agent === undefined) {
      continue;
    }
    entries.push({
      roomId: room.id,
      agentId,
      agentName: agent.name,
      lastActivityAt: room.lastActivityAt,
    });
  }
  return entries.sort(
    (left, right) => (right.lastActivityAt ?? 0) - (left.lastActivityAt ?? 0),
  );
}

/** 相对时间：近一周内用相对表达，更早显示日期。 */
export function formatRelativeTime(
  ms: number | null,
  now = Date.now(),
): string {
  if (ms === null) {
    return "暂无消息";
  }
  const diff = Math.max(0, now - ms);
  const minute = 60_000;
  const hour = 60 * minute;
  const day = 24 * hour;
  if (diff < minute) {
    return "刚刚";
  }
  if (diff < hour) {
    return `${Math.floor(diff / minute)} 分钟前`;
  }
  if (diff < day) {
    return `${Math.floor(diff / hour)} 小时前`;
  }
  if (diff < 7 * day) {
    return `${Math.floor(diff / day)} 天前`;
  }
  return new Date(ms).toLocaleDateString("zh-CN");
}

/** 运行状态点：服务端没有跨房间活动查询，用最近 90 秒的活动近似。 */
const DIRECT_ACTIVE_WINDOW_MS = 90_000;

function hasRecentActivity(ms: number | null, now: number): boolean {
  return ms !== null && now - ms < DIRECT_ACTIVE_WINDOW_MS;
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

function ModelsIcon() {
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
      <path d="M12 3 3 7.5 12 12l9-4.5L12 3Z" />
      <path d="m3 12 9 4.5L21 12" />
      <path d="m3 16.5 9 4.5 9-4.5" />
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
  if (section === "models") {
    return <ModelsIcon />;
  }
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
  directChats,
}: AppSidebarProps) {
  const drawer = variant === "drawer";
  const recentEmpty =
    summary.roomCount === 0
      ? "还没有房间"
      : summary.roomCount === null
        ? "房间数据不可用"
        : "暂无最近房间";

  const location = useLocation();
  const navigate = useNavigate();
  const [agents, setAgents] = useState<Agent[]>([]);
  const [fetchedDirects, setFetchedDirects] = useState<Room[]>([]);
  const [directsCollapsed, setDirectsCollapsed] = useState(() =>
    readSidebarDirectsCollapsed(),
  );
  const [pickerOpen, setPickerOpen] = useState(false);
  const [pickerBusy, setPickerBusy] = useState(false);
  const [pickerError, setPickerError] = useState<string | null>(null);
  const [now, setNow] = useState(() => Date.now());

  // 路由变化后刷新：新建直聊、发送消息都会更新最近活动时间。
  useEffect(() => {
    if (directChats !== undefined) {
      return;
    }
    let cancelled = false;
    void Promise.all([api.agents.list(), api.rooms.list("direct")])
      .then(([agentList, rooms]) => {
        if (!cancelled) {
          setAgents(agentList);
          setFetchedDirects(rooms);
        }
      })
      .catch(() => {
        // 直聊区块失败不影响主导航；保留上一次列表。
      });
    return () => {
      cancelled = true;
    };
  }, [location.pathname, directChats]);

  // 停在当前页面发消息时也要更新相对时间与活动点：30 秒轮询一次。
  useEffect(() => {
    let cancelled = false;
    const timer = window.setInterval(() => {
      setNow(Date.now());
      if (directChats !== undefined) {
        return;
      }
      void Promise.all([api.agents.list(), api.rooms.list("direct")])
        .then(([agentList, rooms]) => {
          if (!cancelled) {
            setAgents(agentList);
            setFetchedDirects(rooms);
          }
        })
        .catch(() => {
          // 轮询失败保留上一次列表，不打断侧栏。
        });
    }, 30_000);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [directChats]);

  const entries =
    directChats ?? buildDirectChatEntries(fetchedDirects, agents);

  const toggleDirects = () => {
    const next = !directsCollapsed;
    setDirectsCollapsed(next);
    writeSidebarDirectsCollapsed(next);
  };

  const startDirectChat = async (agentId: number) => {
    if (pickerBusy) {
      return;
    }
    setPickerBusy(true);
    setPickerError(null);
    try {
      // 幂等接口：已有直聊时返回原房间，不会重复创建。
      const room = await api.agents.directChat.create(agentId);
      setPickerOpen(false);
      navigate(`/chat/room/${room.id}`);
      onNavigate?.();
    } catch (cause) {
      setPickerError(errorMessage(cause));
    } finally {
      setPickerBusy(false);
    }
  };

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
        <div className="app-sidebar-direct">
          <div className="app-sidebar-section-head">
            <button
              type="button"
              className="app-sidebar-section-toggle"
              aria-expanded={!directsCollapsed}
              onClick={toggleDirects}
            >
              <span className="app-sidebar-section-caret" aria-hidden="true">
                {directsCollapsed ? "▸" : "▾"}
              </span>
              对话
            </button>
            <button
              type="button"
              className="app-sidebar-icon-button app-sidebar-direct-add"
              aria-label="发起直聊"
              title="发起直聊"
              aria-expanded={pickerOpen}
              onClick={() => {
                setPickerError(null);
                setPickerOpen((open) => !open);
              }}
            >
              ✎
            </button>
          </div>

          {!directsCollapsed &&
            (entries.length === 0 ? (
              <p className="app-sidebar-empty">点 ✎ 选择智能体开始对话</p>
            ) : (
              <ul className="app-sidebar-direct-list">
                {entries.map((entry) => (
                  <li key={entry.roomId}>
                    <Link
                      to={`/chat/room/${entry.roomId}`}
                      className="app-sidebar-direct-link"
                      onClick={onNavigate}
                      title={`与 ${entry.agentName} 对话`}
                    >
                      <AgentAvatar
                        id={entry.agentId}
                        name={entry.agentName}
                        size="sm"
                      />
                      <span className="app-sidebar-direct-body">
                        <span className="app-sidebar-direct-name">
                          {entry.agentName}
                        </span>
                        <span className="app-sidebar-direct-meta">
                          {formatRelativeTime(entry.lastActivityAt, now)}
                        </span>
                      </span>
                      {hasRecentActivity(entry.lastActivityAt, now) && (
                        <span
                          className="live-dot app-sidebar-direct-dot"
                          title="最近有活动"
                          aria-label="最近有活动"
                        />
                      )}
                    </Link>
                  </li>
                ))}
              </ul>
            ))}

          {pickerOpen && (
            <div
              className="app-sidebar-direct-picker"
              role="group"
              aria-label="选择智能体"
            >
              {pickerError !== null && (
                <p className="app-sidebar-empty">{pickerError}</p>
              )}
              {agents.length === 0 ? (
                <p className="app-sidebar-empty">暂无智能体</p>
              ) : (
                agents.map((agent) => (
                  <button
                    key={agent.id}
                    type="button"
                    className="app-sidebar-direct-choice"
                    onClick={() => void startDirectChat(agent.id)}
                    disabled={pickerBusy}
                  >
                    <AgentAvatar id={agent.id} name={agent.name} size="sm" />
                    <span className="app-sidebar-direct-choice-name">
                      {agent.name}
                    </span>
                  </button>
                ))
              )}
            </div>
          )}
        </div>
      )}

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
