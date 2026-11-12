import type { AgentSessionStatus, SessionOverview } from "@moon/shared";

/** 状态筛选：all 表示不过滤。 */
export type SessionStatusFilter = "all" | AgentSessionStatus;

export const SESSION_STATUS_LABELS: Record<AgentSessionStatus, string> = {
  active: "活动中",
  archived: "已归档",
};

export interface SessionFilterOptions {
  status: SessionStatusFilter;
  query: string;
}

/** 按状态与关键词过滤；关键词匹配房间名、智能体名、harness、会话 ID 与 harness 会话 ID。 */
export function filterSessions(
  sessions: readonly SessionOverview[],
  options: SessionFilterOptions,
): SessionOverview[] {
  const query = options.query.trim().toLowerCase();
  return sessions.filter((session) => {
    if (options.status !== "all" && session.status !== options.status) {
      return false;
    }
    if (query === "") {
      return true;
    }
    return (
      session.roomName.toLowerCase().includes(query) ||
      session.agentName.toLowerCase().includes(query) ||
      session.harness.toLowerCase().includes(query) ||
      `#${session.id}`.includes(query) ||
      (session.harnessSessionId ?? "").toLowerCase().includes(query)
    );
  });
}

export interface SessionStats {
  total: number;
  active: number;
  archived: number;
}

export function sessionStats(sessions: readonly SessionOverview[]): SessionStats {
  let active = 0;
  for (const session of sessions) {
    if (session.status === "active") {
      active += 1;
    }
  }
  return {
    total: sessions.length,
    active,
    archived: sessions.length - active,
  };
}

function pad(value: number): string {
  return value < 10 ? `0${value}` : String(value);
}

/** 最近使用时间：今天/昨天带时分，30 天内 N 天前，更早显示日期；缺失返回 null。 */
export function formatSessionTime(ms: number | null, now: number): string | null {
  if (ms === null) {
    return null;
  }
  const date = new Date(ms);
  const current = new Date(now);
  const startOfDay = (value: Date): number =>
    new Date(value.getFullYear(), value.getMonth(), value.getDate()).getTime();
  const dayDiff = Math.round(
    (startOfDay(current) - startOfDay(date)) / 86_400_000,
  );
  const clock = `${pad(date.getHours())}:${pad(date.getMinutes())}`;
  if (dayDiff === 0) {
    return `今天 ${clock}`;
  }
  if (dayDiff === 1) {
    return `昨天 ${clock}`;
  }
  if (dayDiff > 1 && dayDiff <= 30) {
    return `${dayDiff} 天前`;
  }
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

/** 最近运行的房间运行页路径；从未运行时为 null。 */
export function sessionRunPath(session: SessionOverview): string | null {
  if (session.lastRunId === null) {
    return null;
  }
  return `/rooms/${session.roomId}/runs/${session.lastRunId}`;
}
