import { describe, expect, it } from "vitest";
import type { Task } from "@moon/shared";
import {
  TASK_BOARD_COLUMNS,
  TASK_PRIORITY_LABELS,
  TASK_STATUS_LABELS,
  TASK_TRANSITIONS,
  canActivateTask,
  filterTasksByRoom,
  formatDue,
  formatSchedule,
  fromDatetimeLocal,
  groupTasksByStatus,
  mergeTasks,
  parseTaskView,
  sortTasksByPriority,
  toDatetimeLocal,
} from "./taskView";

function task(overrides: Partial<Task>): Task {
  return {
    id: 1,
    roomId: 1,
    title: "任务",
    description: "",
    status: "backlog",
    priority: "normal",
    assigneeAgentId: null,
    dueAt: null,
    scheduledAt: null,
    plan: null,
    createdByAgentId: null,
    createdAt: 1,
    updatedAt: 1,
    ...overrides,
  };
}

describe("taskView 状态映射", () => {
  it("覆盖全部状态、优先级与流转表", () => {
    expect(Object.keys(TASK_STATUS_LABELS).sort()).toEqual(
      ["backlog", "blocked", "cancelled", "done", "in_progress", "review"].sort(),
    );
    expect(Object.keys(TASK_PRIORITY_LABELS).sort()).toEqual(
      ["high", "low", "normal", "urgent"].sort(),
    );
    expect(TASK_TRANSITIONS.done).toEqual([]);
    expect(TASK_TRANSITIONS.cancelled).toEqual([]);
    expect(TASK_TRANSITIONS.review).toContain("done");
    expect(TASK_TRANSITIONS.blocked).not.toContain("done");
    expect(TASK_TRANSITIONS.backlog).toContain("cancelled");
    expect(TASK_TRANSITIONS.in_progress).toContain("cancelled");
    expect(TASK_TRANSITIONS.review).toContain("cancelled");
    expect(TASK_TRANSITIONS.blocked).toContain("cancelled");
  });

  it("只有待办与进行中允许启动编排，终态不再可启动", () => {
    expect(canActivateTask(task({ status: "backlog" }))).toBe(true);
    expect(canActivateTask(task({ status: "in_progress" }))).toBe(true);
    expect(canActivateTask(task({ status: "review" }))).toBe(false);
    expect(canActivateTask(task({ status: "blocked" }))).toBe(false);
    expect(canActivateTask(task({ status: "done" }))).toBe(false);
    expect(canActivateTask(task({ status: "cancelled" }))).toBe(false);
  });
});

describe("mergeTasks", () => {
  it("合并多房间任务并按更新时间倒序，同时间按 id 倒序", () => {
    const merged = mergeTasks([
      [
        task({ id: 1, roomId: 1, updatedAt: 100 }),
        task({ id: 4, roomId: 1, updatedAt: 300 }),
      ],
      [
        task({ id: 2, roomId: 2, updatedAt: 300 }),
        task({ id: 3, roomId: 2, updatedAt: 200 }),
      ],
    ]);

    expect(merged.map((item) => item.id)).toEqual([4, 2, 3, 1]);
  });

  it("空列表返回空数组", () => {
    expect(mergeTasks([])).toEqual([]);
    expect(mergeTasks([[], []])).toEqual([]);
  });
});

describe("filterTasksByRoom", () => {
  const tasks = [
    task({ id: 1, roomId: 1 }),
    task({ id: 2, roomId: 2 }),
    task({ id: 3, roomId: 1 }),
  ];

  it("按房间过滤且不修改原数组", () => {
    const filtered = filterTasksByRoom(tasks, 1);
    expect(filtered.map((item) => item.id)).toEqual([1, 3]);
    expect(tasks).toHaveLength(3);
  });

  it("null 表示全部房间", () => {
    expect(filterTasksByRoom(tasks, null)).toHaveLength(3);
  });
});

describe("看板分组与排序", () => {
  it("固定五列顺序、空列保留、cancelled 不进入看板", () => {
    const columns = groupTasksByStatus([
      task({ id: 1, status: "backlog" }),
      task({ id: 2, status: "done" }),
      task({ id: 3, status: "cancelled" }),
      task({ id: 4, status: "review" }),
    ]);
    expect(columns.map((column) => column.status)).toEqual([
      "backlog",
      "in_progress",
      "review",
      "blocked",
      "done",
    ]);
    expect(columns.map((column) => column.tasks.length)).toEqual([1, 0, 1, 0, 1]);
    expect(columns.some((column) => column.status === "cancelled")).toBe(false);
    expect(TASK_BOARD_COLUMNS).not.toContain("cancelled");
  });

  it("优先级高在前，同优先级按更新时间倒序，再按 id 倒序", () => {
    const sorted = sortTasksByPriority([
      task({ id: 1, priority: "low", updatedAt: 500 }),
      task({ id: 2, priority: "urgent", updatedAt: 100 }),
      task({ id: 3, priority: "normal", updatedAt: 300 }),
      task({ id: 4, priority: "urgent", updatedAt: 400 }),
      task({ id: 5, priority: "normal", updatedAt: 300 }),
    ]);
    expect(sorted.map((item) => item.id)).toEqual([4, 2, 5, 3, 1]);
  });

  it("不修改入参", () => {
    const tasks = [task({ id: 1, priority: "low" }), task({ id: 2, priority: "urgent" })];
    sortTasksByPriority(tasks);
    expect(tasks.map((item) => item.id)).toEqual([1, 2]);
  });
});

describe("formatDue", () => {
  const now = new Date(2026, 1, 10, 12, 0, 0).getTime();

  it("无截止返回 null", () => {
    expect(formatDue(null, now)).toBeNull();
  });

  it("逾期/今天/明天/未来/日期边界", () => {
    expect(formatDue(new Date(2026, 1, 8, 9).getTime(), now)).toBe("已逾期 2 天");
    expect(formatDue(new Date(2026, 1, 10, 9).getTime(), now)).toBe("已逾期");
    expect(formatDue(new Date(2026, 1, 10, 18).getTime(), now)).toBe("今天到期");
    expect(formatDue(new Date(2026, 1, 11, 9).getTime(), now)).toBe("明天到期");
    expect(formatDue(new Date(2026, 1, 13, 9).getTime(), now)).toBe("3 天后到期");
    expect(formatDue(new Date(2026, 2, 20, 9).getTime(), now)).toBe("2026-03-20");
  });
});

describe("formatSchedule", () => {
  const now = new Date(2026, 1, 10, 12, 0, 0).getTime();

  it("无定时返回 null；已到点显示待启动", () => {
    expect(formatSchedule(null, now)).toBeNull();
    expect(formatSchedule(now, now)).toBe("待启动");
    expect(formatSchedule(now - 60_000, now)).toBe("待启动");
  });

  it("分钟/小时/今天/明天/日期", () => {
    expect(formatSchedule(now + 30_000, now)).toBe("1 分钟内启动");
    expect(formatSchedule(now + 15 * 60_000, now)).toBe("15 分钟后启动");
    expect(formatSchedule(new Date(2026, 1, 10, 18, 30).getTime(), now)).toBe(
      "今天 18:30 启动",
    );
    expect(formatSchedule(new Date(2026, 1, 11, 9, 5).getTime(), now)).toBe(
      "明天 09:05 启动",
    );
    expect(formatSchedule(new Date(2026, 1, 20, 9, 5).getTime(), now)).toBe(
      "2026-02-20 09:05 启动",
    );
  });
});

describe("datetime-local 互转", () => {
  it("空值与 null 互转", () => {
    expect(toDatetimeLocal(null)).toBe("");
    expect(fromDatetimeLocal("  ")).toBeNull();
    expect(fromDatetimeLocal("not-a-date")).toBeNull();
  });

  it("本地时间往返一致（分钟精度）", () => {
    const ms = new Date(2026, 5, 15, 8, 30).getTime();
    expect(toDatetimeLocal(ms)).toBe("2026-06-15T08:30");
    expect(fromDatetimeLocal("2026-06-15T08:30")).toBe(ms);
  });
});

describe("parseTaskView", () => {
  it("只接受 board，其余回退 list", () => {
    expect(parseTaskView("board")).toBe("board");
    expect(parseTaskView("list")).toBe("list");
    expect(parseTaskView(null)).toBe("list");
    expect(parseTaskView("grid")).toBe("list");
  });
});
