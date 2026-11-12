import { useCallback, useEffect, useMemo, useState } from "react";
import { Link } from "react-router-dom";
import type { SessionOverview } from "@moon/shared";
import { api, errorMessage } from "../api/client";
import { EmptyState } from "../components/layout/EmptyState";
import { ListSkeleton } from "../components/layout/ListSkeleton";
import { PageHeader } from "../components/layout/PageHeader";
import {
  SESSION_STATUS_LABELS,
  filterSessions,
  formatSessionTime,
  sessionRunPath,
  sessionStats,
  type SessionStatusFilter,
} from "../components/sessionListView";

const STATUS_OPTIONS: ReadonlyArray<{
  value: SessionStatusFilter;
  label: string;
}> = [
  { value: "all", label: "全部状态" },
  { value: "active", label: "活动中" },
  { value: "archived", label: "已归档" },
];

/**
 * 会话管理页：跨房间列出全部智能体会话。
 *
 * 归档保留存储、下次运行新建会话；删除会同时清理本地存储。两者都复用
 * 房间内既有接口，页面只负责刷新与错误提示。
 */
export default function SessionsPage() {
  const [sessions, setSessions] = useState<SessionOverview[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [pendingId, setPendingId] = useState<number | null>(null);
  const [query, setQuery] = useState("");
  const [status, setStatus] = useState<SessionStatusFilter>("all");

  const load = useCallback(async (): Promise<void> => {
    try {
      setSessions(await api.sessions.listAll());
      setError(null);
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const stats = useMemo(() => sessionStats(sessions), [sessions]);
  const visible = useMemo(
    () => filterSessions(sessions, { query, status }),
    [sessions, query, status],
  );

  const archive = async (session: SessionOverview): Promise<void> => {
    if (
      !window.confirm(
        `归档会话 #${session.id}？归档后下次运行会新建会话，本地存储保留。`,
      )
    ) {
      return;
    }
    setActionError(null);
    setPendingId(session.id);
    try {
      await api.sessions.archive(session.roomId, session.id);
      await load();
    } catch (cause) {
      setActionError(errorMessage(cause));
    } finally {
      setPendingId(null);
    }
  };

  const remove = async (session: SessionOverview): Promise<void> => {
    if (
      !window.confirm(
        `删除会话 #${session.id}？会话记录与本地存储会一起清理，无法恢复。`,
      )
    ) {
      return;
    }
    setActionError(null);
    setPendingId(session.id);
    try {
      await api.sessions.remove(session.roomId, session.id);
      await load();
    } catch (cause) {
      setActionError(errorMessage(cause));
    } finally {
      setPendingId(null);
    }
  };

  return (
    <div className="page">
      <PageHeader
        title="会话"
        subtitle="跨房间查看智能体的本地会话；归档后不再续接，删除会清理本地存储。"
      />

      {error !== null && (
        <p className="banner error" role="alert">
          {error}
          <button
            type="button"
            className="btn btn-ghost btn-sm"
            onClick={() => void load()}
          >
            重试
          </button>
        </p>
      )}

      {actionError !== null && (
        <p className="banner error" role="alert">
          {actionError}
        </p>
      )}

      <div className="sessions-toolbar">
        <label className="sessions-filter">
          <span className="field-label">搜索</span>
          <input
            type="search"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="房间、智能体或会话 ID"
            aria-label="搜索会话"
          />
        </label>

        <label className="sessions-filter">
          <span className="field-label">状态</span>
          <select
            value={status}
            onChange={(event) =>
              setStatus(event.target.value as SessionStatusFilter)
            }
            aria-label="按状态筛选会话"
          >
            {STATUS_OPTIONS.map((option) => (
              <option key={option.value} value={option.value}>
                {option.label}
              </option>
            ))}
          </select>
        </label>

        <span className="muted small sessions-count">
          共 {stats.total} 个 · 活动中 {stats.active} · 已归档 {stats.archived}
        </span>
      </div>

      {loading ? (
        <ListSkeleton variant="task" />
      ) : error !== null ? null : sessions.length === 0 ? (
        <EmptyState
          title="还没有会话"
          description="智能体在房间运行并开启复用会话后，会话会出现在这里。"
          action={
            <Link className="btn btn-primary btn-sm" to="/rooms">
              前往房间
            </Link>
          }
        />
      ) : visible.length === 0 ? (
        <EmptyState
          title="没有匹配的会话"
          description="调整搜索关键词或状态筛选后再试。"
        />
      ) : (
        <div className="sessions-table-wrap">
          <table className="sessions-table">
            <thead>
              <tr>
                <th scope="col">会话</th>
                <th scope="col">房间</th>
                <th scope="col">智能体</th>
                <th scope="col">状态</th>
                <th scope="col">最近使用</th>
                <th scope="col">最近运行</th>
                <th scope="col">操作</th>
              </tr>
            </thead>
            <tbody>
              {visible.map((session) => {
                const lastUsed = formatSessionTime(
                  session.lastUsedAt ?? session.createdAt,
                  Date.now(),
                );
                const runPath = sessionRunPath(session);
                const pending = pendingId === session.id;
                return (
                  <tr key={session.id}>
                    <td>
                      <span className="session-table-id">#{session.id}</span>
                      <span className="session-harness">{session.harness}</span>
                      {session.harnessSessionId !== null && (
                        <span
                          className="session-key"
                          title={session.harnessSessionId}
                        >
                          {session.harnessSessionId.length <= 16
                            ? session.harnessSessionId
                            : `${session.harnessSessionId.slice(0, 12)}…`}
                        </span>
                      )}
                    </td>
                    <td>
                      <Link to={`/rooms/${session.roomId}`}>
                        {session.roomName}
                      </Link>
                    </td>
                    <td>{session.agentName}</td>
                    <td>
                      <span
                        className={`status-pill session-${session.status}`}
                      >
                        {SESSION_STATUS_LABELS[session.status]}
                      </span>
                    </td>
                    <td className="session-time">{lastUsed}</td>
                    <td>
                      {runPath === null ? (
                        <span className="muted">—</span>
                      ) : (
                        <Link to={runPath}>
                          #{session.lastRunId}
                        </Link>
                      )}
                    </td>
                    <td>
                      <div className="session-actions">
                        {session.status === "active" && (
                          <button
                            type="button"
                            className="btn btn-secondary btn-sm"
                            disabled={pending}
                            onClick={() => void archive(session)}
                          >
                            {pending ? "处理中…" : "归档"}
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
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
