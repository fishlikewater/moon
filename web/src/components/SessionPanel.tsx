import { useCallback, useEffect, useMemo, useState } from "react";
import type { AgentSession, RoomSessionsResponse } from "@moon/shared";
import { api, errorMessage } from "../api/client";

export interface SessionPanelMember {
  id: number;
  name: string;
  builtinRole: "moderator" | null;
}

export interface SessionPanelProps {
  roomId: number;
  members: readonly SessionPanelMember[];
  onClose: () => void;
}

function formatDateTime(ms: number): string {
  return new Date(ms).toLocaleString("zh-CN", { hour12: false });
}

/** 会话 ID 只展示可辨识前缀；完整值放在 title 里。 */
function shortSessionId(value: string): string {
  return value.length <= 16 ? value : `${value.slice(0, 12)}…`;
}

/**
 * 房间会话面板。
 *
 * 左侧使用抽屉形态（与任务详情一致）；成员复用开关即时保存，
 * 会话列表支持归档（结束当前会话）与删除（同时清理本地存储）。
 * 加载、空态、错误与操作中禁用都在面板内处理，不改动房间页其他区块。
 */
export default function SessionPanel({
  roomId,
  members,
  onClose,
}: SessionPanelProps) {
  const [data, setData] = useState<RoomSessionsResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [savingAgentIds, setSavingAgentIds] = useState<ReadonlySet<number>>(
    new Set(),
  );
  const [pendingSessionId, setPendingSessionId] = useState<number | null>(null);
  const [showArchived, setShowArchived] = useState(false);

  const load = useCallback(async (): Promise<void> => {
    try {
      const response = await api.sessions.list(roomId);
      setData(response);
      setError(null);
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setLoading(false);
    }
  }, [roomId]);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const response = await api.sessions.list(roomId);
        if (!cancelled) {
          setData(response);
          setError(null);
        }
      } catch (cause) {
        if (!cancelled) {
          setError(errorMessage(cause));
        }
      } finally {
        if (!cancelled) {
          setLoading(false);
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [roomId]);

  useEffect(() => {
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    const handleKeyDown = (event: KeyboardEvent): void => {
      if (event.key === "Escape") {
        onClose();
      }
    };
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.body.style.overflow = previousOverflow;
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [onClose]);

  const settingsByAgent = useMemo(() => {
    const map = new Map<number, boolean>();
    for (const item of data?.members ?? []) {
      map.set(item.agentId, item.reuseSession);
    }
    return map;
  }, [data]);

  const agentNameById = useMemo(() => {
    const map = new Map<number, string>();
    for (const member of members) {
      map.set(member.id, member.name);
    }
    return map;
  }, [members]);

  const visibleSessions = useMemo(() => {
    const sessions = data?.sessions ?? [];
    return showArchived
      ? sessions
      : sessions.filter((session) => session.status === "active");
  }, [data, showArchived]);

  const toggleReuse = async (
    agentId: number,
    reuseSession: boolean,
  ): Promise<void> => {
    setSavingAgentIds((current) => new Set(current).add(agentId));
    setActionError(null);
    try {
      const updated = await api.sessions.setReuse(
        roomId,
        agentId,
        reuseSession,
      );
      setData((current) =>
        current === null
          ? current
          : {
              ...current,
              members: current.members.map((item) =>
                item.agentId === updated.agentId ? updated : item,
              ),
            },
      );
    } catch (cause) {
      setActionError(errorMessage(cause));
    } finally {
      setSavingAgentIds((current) => {
        const next = new Set(current);
        next.delete(agentId);
        return next;
      });
    }
  };

  const archive = async (session: AgentSession): Promise<void> => {
    const name = agentNameById.get(session.agentId) ?? `#${session.agentId}`;
    if (
      !window.confirm(
        `归档 ${name} 的当前会话？下次运行会开启新会话，原会话存储保留。`,
      )
    ) {
      return;
    }

    setPendingSessionId(session.id);
    setActionError(null);
    try {
      const updated = await api.sessions.archive(roomId, session.id);
      setData((current) =>
        current === null
          ? current
          : {
              ...current,
              sessions: current.sessions.map((item) =>
                item.id === updated.id ? updated : item,
              ),
            },
      );
    } catch (cause) {
      setActionError(errorMessage(cause));
    } finally {
      setPendingSessionId(null);
    }
  };

  const remove = async (session: AgentSession): Promise<void> => {
    const name = agentNameById.get(session.agentId) ?? `#${session.agentId}`;
    if (
      !window.confirm(
        `删除 ${name} 的会话 #${session.id} 及其本地存储？此操作不可恢复。`,
      )
    ) {
      return;
    }

    setPendingSessionId(session.id);
    setActionError(null);
    try {
      await api.sessions.remove(roomId, session.id);
      setData((current) =>
        current === null
          ? current
          : {
              ...current,
              sessions: current.sessions.filter(
                (item) => item.id !== session.id,
              ),
            },
      );
    } catch (cause) {
      setActionError(errorMessage(cause));
    } finally {
      setPendingSessionId(null);
    }
  };

  return (
    <>
      <div className="drawer-backdrop" onClick={onClose} aria-hidden="true" />
      <aside
        className="drawer"
        role="dialog"
        aria-modal="true"
        aria-label="会话管理"
      >
        <header className="drawer-head">
          <div className="drawer-title-group">
            <h2 className="drawer-title">会话管理</h2>
            <span className="muted small">按房间隔离</span>
          </div>
          <button
            type="button"
            className="btn btn-ghost btn-sm"
            aria-label="关闭会话管理"
            onClick={onClose}
          >
            关闭
          </button>
        </header>

        <div className="drawer-body">
          {loading && <p className="muted">加载中…</p>}

          {!loading && error !== null && (
            <div>
              <p className="banner error" role="alert">
                {error}
              </p>
              <button
                type="button"
                className="btn btn-secondary btn-sm"
                onClick={() => {
                  setLoading(true);
                  void load();
                }}
              >
                重试
              </button>
            </div>
          )}

          {!loading && error === null && (
            <>
              <section className="session-section">
                <h3 className="session-section-title">成员会话复用</h3>
                <p className="muted small">
                  开启后，该成员在本房间的连续运行会续接同一 harness
                  会话；关闭则每次运行独立。
                </p>
                <ul className="session-member-list">
                  {members.map((member) => {
                    const enabled = settingsByAgent.get(member.id) ?? false;
                    const saving = savingAgentIds.has(member.id);
                    return (
                      <li className="session-member-row" key={member.id}>
                        <span className="session-member-name">
                          {member.name}
                          {member.builtinRole === "moderator" && (
                            <span className="badge badge-moderator">主持人</span>
                          )}
                        </span>
                        <label className="session-switch">
                          <input
                            type="checkbox"
                            role="switch"
                            aria-label={`${member.name} 复用会话`}
                            checked={enabled}
                            disabled={saving}
                            onChange={(event) => {
                              void toggleReuse(
                                member.id,
                                event.target.checked,
                              );
                            }}
                          />
                          <span className="session-switch-track" aria-hidden="true" />
                          <span className="session-switch-label">
                            {saving ? "保存中…" : enabled ? "复用" : "不复用"}
                          </span>
                        </label>
                      </li>
                    );
                  })}
                </ul>
              </section>

              <section className="session-section">
                <div className="session-list-head">
                  <h3 className="session-section-title">会话</h3>
                  <label className="session-filter">
                    <input
                      type="checkbox"
                      checked={showArchived}
                      onChange={(event) =>
                        setShowArchived(event.target.checked)
                      }
                    />
                    显示已归档
                  </label>
                </div>

                {actionError !== null && (
                  <p className="banner error" role="alert">
                    {actionError}
                  </p>
                )}

                {visibleSessions.length === 0 ? (
                  <p className="muted small">
                    {showArchived
                      ? "还没有会话。开启复用后，成员下一次运行会建立会话。"
                      : "还没有活动会话。开启复用后，成员下一次运行会建立会话。"}
                  </p>
                ) : (
                  <ul className="session-list">
                    {visibleSessions.map((session) => {
                      const pending = pendingSessionId === session.id;
                      return (
                        <li className="session-card" key={session.id}>
                          <div className="session-card-main">
                            <div className="session-card-title">
                              <span className="session-agent">
                                {agentNameById.get(session.agentId) ??
                                  `#${session.agentId}`}
                              </span>
                              <span className="badge badge-harness">
                                {session.harness}
                              </span>
                              <span
                                className={`session-status ${
                                  session.status === "active"
                                    ? "active"
                                    : "archived"
                                }`}
                              >
                                {session.status === "active" ? "使用中" : "已归档"}
                              </span>
                            </div>
                            <div className="session-card-meta">
                              <span
                                className="session-id"
                                title={session.harnessSessionId ?? ""}
                              >
                                {session.harnessSessionId === null
                                  ? "等待首次运行"
                                  : shortSessionId(session.harnessSessionId)}
                              </span>
                              <span>
                                创建 {formatDateTime(session.createdAt)}
                              </span>
                              <span>
                                {session.lastUsedAt === null
                                  ? "尚未使用"
                                  : `最后使用 ${formatDateTime(
                                      session.lastUsedAt,
                                    )}`}
                              </span>
                            </div>
                          </div>
                          <div className="session-card-actions">
                            {session.status === "active" && (
                              <button
                                type="button"
                                className="btn btn-secondary btn-sm"
                                disabled={pending}
                                onClick={() => void archive(session)}
                              >
                                归档
                              </button>
                            )}
                            <button
                              type="button"
                              className="btn btn-ghost btn-sm danger-text"
                              disabled={pending}
                              onClick={() => void remove(session)}
                            >
                              删除
                            </button>
                          </div>
                        </li>
                      );
                    })}
                  </ul>
                )}
              </section>
            </>
          )}
        </div>
      </aside>
    </>
  );
}
