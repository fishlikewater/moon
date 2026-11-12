import { useEffect, useState } from "react";
import type {
  Agent,
  TaskStatus,
  TimelineEntry,
  TimelineEntryKind,
} from "@moon/shared";
import { api, errorMessage } from "../api/client";
import { formatModeratorTranscript } from "./moderatorProtocol";

/** 看板风格的状态文案；与服务端状态枚举一一对应。 */
const STATUS_LABELS: Record<TaskStatus, string> = {
  backlog: "待办",
  in_progress: "进行中",
  review: "评审中",
  done: "已完成",
  blocked: "阻塞",
  cancelled: "已取消",
};

/** `updated` 事件 payload.changed 的中文名。 */
const TASK_FIELD_LABELS: Record<string, string> = {
  title: "标题",
  description: "描述",
  priority: "优先级",
  assigneeAgentId: "指派人",
  dueAt: "截止时间",
  scheduledAt: "定时启动",
};

const RUN_STATUS_LABELS: Record<string, string> = {
  running: "运行中",
  succeeded: "成功",
  failed: "失败",
  interrupted: "已中断",
};

const KIND_LABELS: Record<TimelineEntryKind, string> = {
  task_event: "事件",
  message: "消息",
  run: "运行",
};

/** 运行结果摘要的最大展示长度；超出截断，避免侧栏被长文本撑开。 */
const SUMMARY_LIMIT = 240;

export interface TaskTimelineProps {
  taskId: number;
  /** 任务快照变化（如状态流转）时变化，用于触发重新拉取。 */
  refreshToken?: number;
}

function formatTime(ms: number): string {
  return new Date(ms).toLocaleString("zh-CN", { hour12: false });
}

function isTaskStatus(value: unknown): value is TaskStatus {
  return typeof value === "string" && value in STATUS_LABELS;
}

function statusLabel(value: unknown): string {
  return isTaskStatus(value) ? STATUS_LABELS[value] : String(value);
}

function clip(text: string): string {
  const trimmed = text.trim();
  if (trimmed.length <= SUMMARY_LIMIT) {
    return trimmed;
  }
  return `${trimmed.slice(0, SUMMARY_LIMIT)}…`;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function senderLabel(
  entry: Extract<TimelineEntry, { kind: "message" }>,
  agentNames: ReadonlyMap<number, string>,
): string {
  const { message } = entry;
  if (message.senderType === "user") {
    return "用户";
  }
  if (message.senderType === "system") {
    return "系统";
  }
  if (message.senderAgentId === null) {
    return "智能体";
  }
  return (
    agentNames.get(message.senderAgentId) ?? `智能体#${message.senderAgentId}`
  );
}

function describeTaskEvent(
  entry: Extract<TimelineEntry, { kind: "task_event" }>,
  agentNames: ReadonlyMap<number, string>,
): { text: string; detail: string | null } {
  const payload = entry.event.payload ?? {};

  if (entry.event.type === "created") {
    const source =
      payload.source === "moderator" ? "主持人创建 · " : "";
    return {
      text: `创建任务（${source}${statusLabel(payload.status)}）`,
      detail: null,
    };
  }

  if (entry.event.type === "status_changed") {
    return {
      text: `状态变更：${statusLabel(payload.from)} → ${statusLabel(payload.to)}`,
      detail: null,
    };
  }

  if (entry.event.type === "updated") {
    const changed = Array.isArray(payload.changed)
      ? payload.changed.filter((item): item is string => typeof item === "string")
      : [];
    const labels = changed.map((field) => TASK_FIELD_LABELS[field] ?? field);
    return {
      text: labels.length === 0 ? "任务更新" : `任务更新：${labels.join("、")}`,
      detail: null,
    };
  }

  if (entry.event.type === "scheduled_started") {
    const scheduledAt =
      typeof payload.scheduledAt === "number" ? payload.scheduledAt : null;
    return {
      text:
        scheduledAt === null
          ? "定时启动"
          : `定时启动（原定 ${formatTime(scheduledAt)}）`,
      detail: null,
    };
  }

  if (entry.event.type === "schedule_failed") {
    const reason =
      typeof payload.reason === "string" ? payload.reason : "未知原因";
    return { text: `定时启动失败：${reason}`, detail: null };
  }

  if (entry.event.type === "moderator_plan") {
    const round =
      typeof payload.round === "number" ? payload.round : null;
    const assignments = Array.isArray(payload.assignments)
      ? payload.assignments
      : [];
    const lines = assignments.map((item, index) => {
      if (!isRecord(item)) {
        return `${index + 1}. 无法解析的分配单`;
      }
      const kind = item.kind === "review" ? "复核" : "执行";
      const agentId =
        typeof item.agentId === "number" ? item.agentId : null;
      const name =
        agentId === null
          ? "未知成员"
          : (agentNames.get(agentId) ?? `智能体#${agentId}`);
      const instruction =
        typeof item.instruction === "string" ? item.instruction : "（无指令）";
      const expected =
        typeof item.expectedOutput === "string"
          ? ` → ${item.expectedOutput}`
          : "";
      return `${index + 1}. [${kind}] ${name}：${instruction}${expected}`;
    });

    return {
      text: `主持人计划${round === null ? "" : ` · 第 ${round} 轮`}`,
      detail: lines.length === 0 ? null : clip(lines.join("\n")),
    };
  }

  if (entry.event.type === "moderator_done") {
    const round =
      typeof payload.round === "number" ? payload.round : null;
    const summary =
      typeof payload.summary === "string" ? payload.summary : "（无摘要）";
    const verification = Array.isArray(payload.verification)
      ? payload.verification.filter(
          (item): item is string => typeof item === "string",
        )
      : [];
    const detail = [
      summary,
      ...(verification.length === 0
        ? []
        : ["验收证据：", ...verification.map((item, index) => `${index + 1}. ${item}`)]),
    ].join("\n");

    return {
      text: `主持人验收${round === null ? "" : ` · 第 ${round} 轮`}`,
      detail: clip(detail),
    };
  }

  return { text: entry.event.type, detail: null };
}

function describeEntry(
  entry: TimelineEntry,
  agentNames: ReadonlyMap<number, string>,
  moderatorIds: ReadonlySet<number>,
): { text: string; detail: string | null } {
  if (entry.kind === "task_event") {
    return describeTaskEvent(entry, agentNames);
  }

  if (entry.kind === "run") {
    const run = entry.run;
    const name = agentNames.get(run.agentId) ?? `智能体#${run.agentId}`;
    const status = RUN_STATUS_LABELS[run.status] ?? run.status;
    const summary =
      run.error ??
      (run.result === null
        ? run.prompt
        : formatModeratorTranscript(
            run.result,
            (agentId) => agentNames.get(agentId),
            moderatorIds.has(run.agentId),
          ));
    return {
      text: `运行 · ${name} · ${status}`,
      detail: summary === null || summary.trim() === "" ? null : clip(summary),
    };
  }

  const { message } = entry;
  const status =
    message.status === "final" ? "" : `（${message.status === "failed" ? "失败" : "流式"}）`;
  const content = formatModeratorTranscript(
    message.content,
    (agentId) => agentNames.get(agentId),
    message.senderAgentId !== null && moderatorIds.has(message.senderAgentId),
  );
  return {
    text: `${senderLabel(entry, agentNames)}${status}：${
      content.trim() === "" ? "（无内容）" : clip(content)
    }`,
    detail: null,
  };
}

/**
 * 任务历史时间线（展示组件）。
 *
 * 数据来自服务端聚合接口，按任务 id 拉取，因此页面刷新或重启后仍能完整回放；
 * 不承担状态流转，只在任务快照变化或手动刷新时重新拉取。
 */
export default function TaskTimeline({
  taskId,
  refreshToken = 0,
}: TaskTimelineProps) {
  const [entries, setEntries] = useState<TimelineEntry[]>([]);
  const [agentNames, setAgentNames] = useState<ReadonlyMap<number, string>>(
    new Map(),
  );
  const [moderatorIds, setModeratorIds] = useState<ReadonlySet<number>>(
    new Set(),
  );
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [reloadKey, setReloadKey] = useState(0);

  useEffect(() => {
    let cancelled = false;

    setLoading(true);
    setError(null);

    void (async () => {
      try {
        // 智能体名称只影响可读性，拉取失败不阻断时间线本身。
        const [timeline, agents] = await Promise.all([
          api.tasks.timeline(taskId),
          api.agents.list().catch((): Agent[] => []),
        ]);
        if (cancelled) {
          return;
        }
        setEntries(timeline.entries);
        setAgentNames(
          new Map(agents.map((agent) => [agent.id, agent.name] as const)),
        );
        setModeratorIds(
          new Set(
            agents
              .filter((agent) => agent.builtinRole === "moderator")
              .map((agent) => agent.id),
          ),
        );
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
  }, [taskId, refreshToken, reloadKey]);

  return (
    <section className="timeline" aria-label="任务时间线">
      <header className="timeline-head">
        <h3>时间线</h3>
        <button
          type="button"
          className="btn btn-ghost btn-sm"
          disabled={loading}
          onClick={() => setReloadKey((key) => key + 1)}
        >
          {loading ? "加载中…" : "刷新"}
        </button>
      </header>

      {error !== null ? (
        <div className="timeline-error" role="alert">
          <p>{error}</p>
          <button
            type="button"
            className="btn btn-secondary btn-sm"
            onClick={() => setReloadKey((key) => key + 1)}
          >
            重试
          </button>
        </div>
      ) : loading ? (
        <p className="timeline-muted">时间线加载中…</p>
      ) : entries.length === 0 ? (
        <p className="timeline-muted">该任务还没有时间线条目。</p>
      ) : (
        <ol className="timeline-list">
          {entries.map((entry) => {
            const { text, detail } = describeEntry(
              entry,
              agentNames,
              moderatorIds,
            );
            return (
              <li
                key={`${entry.kind}-${entry.id}`}
                className={`timeline-item ${entry.kind}`}
              >
                <div className="timeline-card">
                  <div className="timeline-meta">
                    <span className="timeline-kind">
                      {KIND_LABELS[entry.kind]}
                    </span>
                    <time>{formatTime(entry.createdAt)}</time>
                  </div>
                  <p className="timeline-text">{text}</p>
                  {detail !== null && (
                    <p className="timeline-detail">{detail}</p>
                  )}
                </div>
              </li>
            );
          })}
        </ol>
      )}
    </section>
  );
}
