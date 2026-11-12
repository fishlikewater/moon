import { describe, expect, it } from "vitest";
import type { SessionOverview } from "@moon/shared";
import {
  SESSION_STATUS_LABELS,
  filterSessions,
  formatSessionTime,
  sessionRunPath,
  sessionStats,
} from "./sessionListView";

function session(overrides: Partial<SessionOverview>): SessionOverview {
  return {
    id: 1,
    roomId: 1,
    roomName: "评审房间",
    agentId: 2,
    agentName: "代码评审员",
    harness: "zcode",
    harnessSessionId: null,
    status: "active",
    createdAt: 1,
    lastUsedAt: null,
    lastRunId: null,
    ...overrides,
  };
}

describe("SESSION_STATUS_LABELS", () => {
  it("覆盖两种状态", () => {
    expect(SESSION_STATUS_LABELS.active).toBe("活动中");
    expect(SESSION_STATUS_LABELS.archived).toBe("已归档");
  });
});

describe("filterSessions", () => {
  const sessions = [
    session({ id: 11, roomName: "评审房间", agentName: "评审员", status: "active" }),
    session({
      id: 22,
      roomName: "文案房间",
      agentName: "文案编辑",
      status: "archived",
      harness: "codex",
      harnessSessionId: "sess_abc123",
    }),
  ];

  it("状态筛选", () => {
    expect(
      filterSessions(sessions, { status: "active", query: "" }).map((item) => item.id),
    ).toEqual([11]);
    expect(
      filterSessions(sessions, { status: "archived", query: "" }).map((item) => item.id),
    ).toEqual([22]);
    expect(filterSessions(sessions, { status: "all", query: "" })).toHaveLength(2);
  });

  it("关键词匹配房间、智能体、会话 ID 与 harness 会话 ID", () => {
    expect(
      filterSessions(sessions, { status: "all", query: "评审" }).map((item) => item.id),
    ).toEqual([11]);
    expect(
      filterSessions(sessions, { status: "all", query: "#22" }).map((item) => item.id),
    ).toEqual([22]);
    expect(
      filterSessions(sessions, { status: "all", query: "SESS_ABC" }).map((item) => item.id),
    ).toEqual([22]);
    expect(
      filterSessions(sessions, { status: "all", query: "codex" }).map((item) => item.id),
    ).toEqual([22]);
  });

  it("空关键词忽略空白且不修改入参", () => {
    expect(filterSessions(sessions, { status: "all", query: "   " })).toHaveLength(2);
    expect(sessions).toHaveLength(2);
  });
});

describe("sessionStats", () => {
  it("统计总数与活动/归档", () => {
    expect(
      sessionStats([
        session({ id: 1, status: "active" }),
        session({ id: 2, status: "active" }),
        session({ id: 3, status: "archived" }),
      ]),
    ).toEqual({ total: 3, active: 2, archived: 1 });
    expect(sessionStats([])).toEqual({ total: 0, active: 0, archived: 0 });
  });
});

describe("formatSessionTime", () => {
  const now = new Date(2026, 1, 10, 12, 0, 0).getTime();

  it("缺失返回 null", () => {
    expect(formatSessionTime(null, now)).toBeNull();
  });

  it("今天/昨天/天数/日期边界", () => {
    expect(formatSessionTime(new Date(2026, 1, 10, 9, 5).getTime(), now)).toBe(
      "今天 09:05",
    );
    expect(formatSessionTime(new Date(2026, 1, 9, 23, 59).getTime(), now)).toBe(
      "昨天 23:59",
    );
    expect(formatSessionTime(new Date(2026, 1, 5, 8, 0).getTime(), now)).toBe(
      "5 天前",
    );
    expect(formatSessionTime(new Date(2025, 11, 1, 8, 0).getTime(), now)).toBe(
      "2025-12-01",
    );
  });
});

describe("sessionRunPath", () => {
  it("有最近运行时指向房间运行页，否则 null", () => {
    expect(sessionRunPath(session({ roomId: 7, lastRunId: 42 }))).toBe(
      "/rooms/7/runs/42",
    );
    expect(sessionRunPath(session({ lastRunId: null }))).toBeNull();
  });
});
