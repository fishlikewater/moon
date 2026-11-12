import type { Task, TaskPriority, TaskStatus } from "@moon/shared";

export const TASK_STATUS_LABELS: Record<TaskStatus, string> = {
  backlog: "待办",
  in_progress: "进行中",
  review: "评审中",
  done: "已完成",
  blocked: "阻塞",
  cancelled: "已取消",
};

export const TASK_PRIORITY_LABELS: Record<TaskPriority, string> = {
  low: "低",
  normal: "普通",
  high: "高",
  urgent: "紧急",
};

/** 看板列顺序；cancelled 是终态，只在列表视图展示。 */
export const TASK_BOARD_COLUMNS: readonly TaskStatus[] = [
  "backlog",
  "in_progress",
  "review",
  "blocked",
  "done",
];

/**
 * 任务状态机允许的流转表。
 *
 * 与服务端 `services/tasks.rs` 的 `is_allowed_transition` 保持一致；前端只据此
 * 渲染合法动作，服务端仍是权威校验方（非法迁移返回 409）。
 */
export const TASK_TRANSITIONS: Record<TaskStatus, readonly TaskStatus[]> = {
  backlog: ["in_progress", "blocked", "cancelled"],
  in_progress: ["review", "blocked", "backlog", "cancelled"],
  review: ["done", "in_progress", "blocked", "cancelled"],
  done: [],
  blocked: ["in_progress", "backlog", "cancelled"],
  cancelled: [],
};

export const TASK_TRANSITION_LABELS: Record<TaskStatus, string> = {
  backlog: "退回待办",
  in_progress: "开始处理",
  review: "提交评审",
  done: "标记完成",
  blocked: "标记阻塞",
  cancelled: "取消任务",
};

const PRIORITY_ORDER: Record<TaskPriority, number> = {
  urgent: 0,
  high: 1,
  normal: 2,
  low: 3,
};

/** 与服务端 activate 的准入一致：只有 backlog / in_progress 可启动编排。 */
export function canActivateTask(task: Task): boolean {
  return task.status === "backlog" || task.status === "in_progress";
}

/**
 * 合并多个房间的任务并按最近更新排序。
 *
 * 跨房间列表没有服务端聚合接口，页面按房间拉取后在这里统一排序；更新时间相同
 * 时按 id 倒序，保证顺序稳定。
 */
export function mergeTasks(lists: readonly (readonly Task[])[]): Task[] {
  return lists
    .flat()
    .sort((left, right) =>
      right.updatedAt === left.updatedAt
        ? right.id - left.id
        : right.updatedAt - left.updatedAt,
    );
}

/** 房间筛选：null 表示全部房间。 */
export function filterTasksByRoom(
  tasks: readonly Task[],
  roomId: number | null,
): Task[] {
  return roomId === null
    ? [...tasks]
    : tasks.filter((task) => task.roomId === roomId);
}

/** 看板排序：优先级高在前，同优先级按最近更新。 */
export function sortTasksByPriority(tasks: readonly Task[]): Task[] {
  return [...tasks].sort(
    (left, right) =>
      PRIORITY_ORDER[left.priority] - PRIORITY_ORDER[right.priority] ||
      right.updatedAt - left.updatedAt ||
      right.id - left.id,
  );
}

export interface TaskBoardColumn {
  status: TaskStatus;
  tasks: Task[];
}

/** 固定五列分组（空列保留）；cancelled 不进入看板。 */
export function groupTasksByStatus(tasks: readonly Task[]): TaskBoardColumn[] {
  return TASK_BOARD_COLUMNS.map((status) => ({
    status,
    tasks: sortTasksByPriority(tasks.filter((task) => task.status === status)),
  }));
}

const MINUTE = 60_000;
const HOUR = 3_600_000;
const DAY = 86_400_000;

function startOfDay(ms: number): number {
  const date = new Date(ms);
  date.setHours(0, 0, 0, 0);
  return date.getTime();
}

function pad(value: number): string {
  return String(value).padStart(2, "0");
}

function formatDateTime(ms: number): string {
  const date = new Date(ms);
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

/**
 * 截止时间文案：已逾期 N 天 / 今天到期 / 明天到期 / N 天后到期 / 具体日期。
 * 传入 now 便于测试，页面每次渲染现算。
 */
export function formatDue(ms: number | null, nowMs: number): string | null {
  if (ms === null) {
    return null;
  }
  if (ms < nowMs) {
    const lateDays = Math.floor((startOfDay(nowMs) - startOfDay(ms)) / DAY);
    return lateDays >= 1 ? `已逾期 ${lateDays} 天` : "已逾期";
  }
  const dayDiff = Math.floor((startOfDay(ms) - startOfDay(nowMs)) / DAY);
  if (dayDiff === 0) {
    return "今天到期";
  }
  if (dayDiff === 1) {
    return "明天到期";
  }
  if (dayDiff <= 7) {
    return `${dayDiff} 天后到期`;
  }
  const date = new Date(ms);
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

/** 定时启动文案：待启动（已到点）/ N 分钟后启动 / 今天 HH:mm 启动 / 日期。 */
export function formatSchedule(ms: number | null, nowMs: number): string | null {
  if (ms === null) {
    return null;
  }
  if (ms <= nowMs) {
    return "待启动";
  }
  const diff = ms - nowMs;
  if (diff < MINUTE) {
    return "1 分钟内启动";
  }
  if (diff < HOUR) {
    return `${Math.floor(diff / MINUTE)} 分钟后启动`;
  }
  const dueDay = startOfDay(ms);
  const nowDay = startOfDay(nowMs);
  const hours = pad(new Date(ms).getHours());
  const minutes = pad(new Date(ms).getMinutes());
  if (dueDay === nowDay) {
    return `今天 ${hours}:${minutes} 启动`;
  }
  if (dueDay - nowDay === DAY) {
    return `明天 ${hours}:${minutes} 启动`;
  }
  return `${formatDateTime(ms)} 启动`;
}

/** epoch 毫秒 → `datetime-local` 输入值（本地时区）。 */
export function toDatetimeLocal(ms: number | null): string {
  if (ms === null) {
    return "";
  }
  const date = new Date(ms);
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

/** `datetime-local` 输入值 → epoch 毫秒；空串或非法值返回 null（本地时区解析）。 */
export function fromDatetimeLocal(value: string): number | null {
  if (value.trim() === "") {
    return null;
  }
  const parsed = new Date(value).getTime();
  return Number.isFinite(parsed) ? parsed : null;
}

export type TaskView = "list" | "board";

export function parseTaskView(value: string | null): TaskView {
  return value === "board" ? "board" : "list";
}
