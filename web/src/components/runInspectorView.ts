import type {
  Agent,
  AgentRun,
  Message,
  RunActivity,
  RunActivityKind,
  RunStatus,
} from "@moon/shared";
import { formatElapsed } from "./messageView";

/** 运行状态中文标签；与任务状态区分，避免复用看板文案。 */
export const RUN_STATUS_LABELS: Record<RunStatus, string> = {
  running: "运行中",
  succeeded: "已完成",
  failed: "失败",
  interrupted: "已中断",
};

const ACTIVITY_KIND_LABELS: Record<RunActivityKind, string> = {
  reasoning: "思考",
  tool: "工具",
  usage: "用量",
  status: "提示",
};

/** 抽屉里默认展示的明细长度；更长的内容折叠到 details 内。 */
const ACTIVITY_PREVIEW_LIMIT = 180;

export function runStatusLabel(status: RunStatus): string {
  return RUN_STATUS_LABELS[status];
}

export function activityKindLabel(kind: RunActivityKind): string {
  return ACTIVITY_KIND_LABELS[kind];
}

/** 模型展示：未配置模型时退回 harness 默认，并附上思考等级。 */
export function describeAgentModel(agent: Agent | undefined): string {
  if (agent === undefined) {
    return "未知智能体";
  }
  const model =
    typeof agent.modelConfig?.model === "string" &&
    agent.modelConfig.model.trim() !== ""
      ? agent.modelConfig.model
      : "默认模型";
  const level =
    typeof agent.modelConfig?.reasoningLevel === "string" &&
    agent.modelConfig.reasoningLevel.trim() !== ""
      ? agent.modelConfig.reasoningLevel
      : null;
  return level === null ? model : `${model} · ${level}`;
}

/** 运行耗时：结束时用 finishedAt，运行中用当前时间。 */
export function formatRunDuration(run: AgentRun, now: number): string {
  const end = run.finishedAt ?? now;
  return formatElapsed(Math.max(0, end - run.createdAt));
}

/** 活动相对运行开始的偏移；用于判断「第几秒调用了什么」。 */
export function formatActivityOffset(
  activityAt: number,
  runCreatedAt: number,
): string {
  const seconds = Math.max(0, (activityAt - runCreatedAt) / 1000);
  if (seconds < 60) {
    return `+${seconds.toFixed(1)}s`;
  }
  const minutes = Math.floor(seconds / 60);
  const rest = Math.floor(seconds % 60);
  return `+${minutes}m${String(rest).padStart(2, "0")}s`;
}

/**
 * 合并服务端快照与本地实时活动。
 *
 * 以 `seq` 去重；同一 seq 优先保留传入顺序更靠后的项（通常是最新实时值），
 * 结果按 seq 升序，保证刷新、重连与实时追加后顺序稳定。
 */
export function mergeActivities(
  fetched: readonly RunActivity[],
  live: readonly RunActivity[],
): RunActivity[] {
  const bySeq = new Map<number, RunActivity>();
  for (const activity of fetched) {
    bySeq.set(activity.seq, activity);
  }
  for (const activity of live) {
    bySeq.set(activity.seq, activity);
  }
  return [...bySeq.values()].sort((left, right) => left.seq - right.seq);
}

/** 运行详情时间线条目：阶段/结论消息与工具活动按发生时间交错。 */
export type RunTimelineEntry =
  | { kind: "message"; key: string; at: number; message: Message }
  | { kind: "activity"; key: string; at: number; activity: RunActivity };

/**
 * 把运行消息与活动合成一条时间线。
 *
 * 同一毫秒内消息优先于活动：阶段总结在工具调用前落定，先读总结再看工具更自然。
 * 活动按 seq 去重，消息按 id 去重，避免快照与实时推送重复。
 */
export function buildRunTimeline(
  messages: readonly Message[],
  activities: readonly RunActivity[],
): RunTimelineEntry[] {
  const entries: RunTimelineEntry[] = [];
  const seenMessages = new Set<number>();
  for (const message of messages) {
    if (seenMessages.has(message.id)) {
      continue;
    }
    seenMessages.add(message.id);
    entries.push({
      kind: "message",
      key: `message-${message.id}`,
      at: message.createdAt,
      message,
    });
  }
  const seenActivities = new Set<number>();
  for (const activity of activities) {
    if (seenActivities.has(activity.seq)) {
      continue;
    }
    seenActivities.add(activity.seq);
    entries.push({
      kind: "activity",
      key: `activity-${activity.seq}`,
      at: activity.at,
      activity,
    });
  }
  return entries.sort((left, right) => {
    if (left.at !== right.at) {
      return left.at - right.at;
    }
    const priority = (entry: RunTimelineEntry): number =>
      entry.kind === "message" ? 0 : 1;
    return priority(left) - priority(right);
  });
}

/** 明细预览：压缩空白；超过上限时截断并标记可展开。 */
export function activityDetailPreview(
  detail: string | null,
  limit = ACTIVITY_PREVIEW_LIMIT,
): { text: string; truncated: boolean } {
  if (detail === null) {
    return { text: "", truncated: false };
  }
  const compact = detail.replace(/\s+/g, " ").trim();
  if (compact.length <= limit) {
    return { text: compact, truncated: false };
  }
  return { text: `${compact.slice(0, limit)}…`, truncated: true };
}

/** 抽屉列表项：运行中的运行与历史运行按 runId 去重后再排序。 */
export interface RunListItem {
  runId: number;
  agentId: number;
  startedAt: number;
  run: AgentRun | null;
  /** 是否仍在页面的 activeRuns 中（优先于 run.status 的降级判断）。 */
  active: boolean;
}

export function buildRunList(
  activeRuns: readonly { runId: number; agentId: number; startedAt: number }[],
  recentRuns: readonly AgentRun[],
): RunListItem[] {
  const items = new Map<number, RunListItem>();
  for (const run of recentRuns) {
    items.set(run.id, {
      runId: run.id,
      agentId: run.agentId,
      startedAt: run.createdAt,
      run,
      active: false,
    });
  }
  for (const active of activeRuns) {
    const existing = items.get(active.runId);
    items.set(active.runId, {
      runId: active.runId,
      agentId: active.agentId,
      startedAt: existing?.startedAt ?? active.startedAt,
      run: existing?.run ?? null,
      active: true,
    });
  }
  return [...items.values()].sort((left, right) => {
    if (left.active !== right.active) {
      return left.active ? -1 : 1;
    }
    return right.startedAt - left.startedAt;
  });
}
