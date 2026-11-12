import { useCallback, useEffect, useMemo, useState } from "react";
import { Link, useNavigate, useParams } from "react-router-dom";
import type {
  Agent,
  AgentRun,
  Message,
  Room,
  RoomEvent,
  RunActivity,
  RunDetailResponse,
} from "@moon/shared";
import { api, errorMessage } from "../api/client";
import {
  RunDetailPanel,
  type ActiveRunSummary,
} from "../components/RunDetailPanel";
import { PageHeader } from "../components/layout/PageHeader";
import { mergeActivities } from "../components/runInspectorView";
import { useRoomEvents } from "../ws/useRoomEvents";

const RUN_LIST_LIMIT = 50;
const ACTIVITY_LIMIT = 300;

function parsePositiveId(value: string | undefined): number | null {
  if (value === undefined || !/^\d+$/.test(value)) {
    return null;
  }
  const parsed = Number(value);
  return Number.isSafeInteger(parsed) && parsed > 0 ? parsed : null;
}

function isEventForRoom(event: RoomEvent, roomId: number | null): boolean {
  return roomId !== null && event.roomId === roomId;
}

/** 按 id 合并运行消息；同一消息的终态覆盖流式占位。 */
function mergeRunMessage(
  messages: readonly Message[],
  incoming: Message,
): Message[] {
  const index = messages.findIndex((message) => message.id === incoming.id);
  if (index === -1) {
    return [...messages, incoming].sort((left, right) => left.seq - right.seq);
  }
  const next = [...messages];
  next[index] = incoming;
  return next;
}

/**
 * 运行详情独立页面。
 *
 * 选中运行完全由 URL 决定（可刷新、可分享）；活动优先取服务端快照，
 * 再按 seq 合并页面会话期间通过 WebSocket 收到的新增活动。
 */
export default function RunPage() {
  const { id, runId: runIdParam } = useParams<{
    id: string;
    runId?: string;
  }>();
  const navigate = useNavigate();
  const roomId = parsePositiveId(id);
  const urlRunId = parsePositiveId(runIdParam);

  const [room, setRoom] = useState<Room | null>(null);
  const [agents, setAgents] = useState<Agent[]>([]);
  const [runs, setRuns] = useState<AgentRun[]>([]);
  const [detail, setDetail] = useState<RunDetailResponse | null>(null);
  const [activitiesByRun, setActivitiesByRun] = useState<
    Record<number, RunActivity[]>
  >({});
  const [messagesByRun, setMessagesByRun] = useState<
    Record<number, Message[]>
  >({});
  const [listLoading, setListLoading] = useState(true);
  const [detailLoading, setDetailLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [detailError, setDetailError] = useState<string | null>(null);
  const [now, setNow] = useState(() => Date.now());

  const selectedRunId = urlRunId ?? runs[0]?.id ?? null;

  const reloadRuns = useCallback(async () => {
    if (roomId === null) {
      return;
    }
    try {
      setRuns(await api.runs.list(roomId, RUN_LIST_LIMIT));
    } catch (cause) {
      setError(errorMessage(cause));
    }
  }, [roomId]);

  useEffect(() => {
    if (roomId === null) {
      setListLoading(false);
      setError("非法的房间地址");
      return;
    }
    let cancelled = false;
    setListLoading(true);
    setError(null);
    setDetail(null);
    setRuns([]);
    setMessagesByRun({});
    void (async () => {
      try {
        const [roomList, agentList, runList] = await Promise.all([
          api.rooms.list(),
          api.agents.list(),
          api.runs.list(roomId, RUN_LIST_LIMIT),
        ]);
        if (cancelled) {
          return;
        }
        setRoom(roomList.find((candidate) => candidate.id === roomId) ?? null);
        setAgents(agentList);
        setRuns(runList);
      } catch (cause) {
        if (!cancelled) {
          setError(errorMessage(cause));
        }
      } finally {
        if (!cancelled) {
          setListLoading(false);
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [roomId]);

  // 直接访问列表页时，自动定位到最近一次运行，保持 URL 可刷新。
  useEffect(() => {
    if (!listLoading && urlRunId === null && runs.length > 0) {
      navigate(`/rooms/${runs[0].roomId}/runs/${runs[0].id}`, {
        replace: true,
      });
    }
  }, [listLoading, navigate, runs, urlRunId]);

  useEffect(() => {
    if (roomId === null || urlRunId === null) {
      setDetail(null);
      setDetailError(null);
      return;
    }
    let cancelled = false;
    setDetailLoading(true);
    setDetailError(null);
    void (async () => {
      try {
        const response = await api.runs.detail(roomId, urlRunId);
        if (!cancelled) {
          setDetail(response);
        }
      } catch (cause) {
        if (!cancelled) {
          setDetail(null);
          setDetailError(errorMessage(cause));
        }
      } finally {
        if (!cancelled) {
          setDetailLoading(false);
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [roomId, urlRunId]);

  const handleEvent = useCallback(
    (event: RoomEvent) => {
      if (!isEventForRoom(event, roomId)) {
        return;
      }
      if (event.type === "run_activity") {
        const activity = event.activity;
        setActivitiesByRun((previous) => {
          const list = previous[activity.runId] ?? [];
          if (list.some((item) => item.seq === activity.seq)) {
            return previous;
          }
          return {
            ...previous,
            [activity.runId]: [...list, activity].slice(-ACTIVITY_LIMIT),
          };
        });
        return;
      }
      if (event.type === "message_started") {
        setMessagesByRun((previous) => ({
          ...previous,
          [event.runId]: mergeRunMessage(
            previous[event.runId] ?? [],
            event.message,
          ),
        }));
        return;
      }
      if (event.type === "message_delta") {
        setMessagesByRun((previous) => {
          let changed = false;
          const next: Record<number, Message[]> = { ...previous };
          for (const [runId, list] of Object.entries(previous)) {
            if (!list.some((message) => message.id === event.messageId)) {
              continue;
            }
            changed = true;
            next[Number(runId)] = list.map((message) =>
              message.id === event.messageId
                ? { ...message, content: message.content + event.text }
                : message,
            );
          }
          return changed ? next : previous;
        });
        return;
      }
      if (event.type === "message_final") {
        if (event.message.runId !== null) {
          const runId = event.message.runId;
          setMessagesByRun((previous) => ({
            ...previous,
            [runId]: mergeRunMessage(previous[runId] ?? [], event.message),
          }));
        }
        void reloadRuns();
        return;
      }
      // 运行列表的状态与输出由服务端事实决定：有开始就重新拉取。
      if (event.type === "run_started") {
        // 首个占位消息没有 message_started，先合并才能接收后续 delta。
        setMessagesByRun((previous) => ({
          ...previous,
          [event.runId]: mergeRunMessage(
            previous[event.runId] ?? [],
            event.message,
          ),
        }));
        void reloadRuns();
      }
    },
    [reloadRuns, roomId],
  );

  const { connected } = useRoomEvents(roomId, handleEvent);

  const hasRunning =
    runs.some((run) => run.status === "running") ||
    detail?.run.status === "running";

  useEffect(() => {
    if (!hasRunning) {
      return;
    }
    setNow(Date.now());
    const timer = setInterval(() => setNow(Date.now()), 1_000);
    return () => clearInterval(timer);
  }, [hasRunning]);

  const agentById = useMemo(() => {
    const map = new Map<number, Agent>();
    for (const agent of agents) {
      map.set(agent.id, agent);
    }
    return map;
  }, [agents]);

  const activeRuns = useMemo<ActiveRunSummary[]>(
    () =>
      runs
        .filter((run) => run.status === "running")
        .map((run) => ({
          runId: run.id,
          agentId: run.agentId,
          startedAt: run.createdAt,
        })),
    [runs],
  );

  const activities = useMemo(() => {
    if (selectedRunId === null) {
      return [];
    }
    const fetched =
      detail?.run.id === selectedRunId ? detail.activities : [];
    return mergeActivities(fetched, activitiesByRun[selectedRunId] ?? []);
  }, [selectedRunId, detail, activitiesByRun]);

  const messages = useMemo(() => {
    if (selectedRunId === null) {
      return [];
    }
    const fetched =
      detail?.run.id === selectedRunId ? detail.messages : [];
    const merged = [...fetched];
    for (const message of messagesByRun[selectedRunId] ?? []) {
      const index = merged.findIndex((item) => item.id === message.id);
      if (index === -1) {
        merged.push(message);
      } else {
        merged[index] = message;
      }
    }
    return merged.sort((left, right) => left.seq - right.seq);
  }, [selectedRunId, detail, messagesByRun]);

  return (
    <section className="page run-page page-fixed">
      <PageHeader
        backTo={`/rooms/${roomId ?? ""}`}
        backLabel="返回房间"
        title="运行详情"
        meta={
          <p className="path">
            {room?.name ?? (roomId === null ? "房间不可用" : `房间 #${roomId}`)}
          </p>
        }
        actions={
          <>
            <Link
              className="btn btn-secondary btn-sm"
              to={`/rooms/${roomId ?? ""}`}
            >
              房间聊天
            </Link>
            <Link className="btn btn-secondary btn-sm" to="/tasks">
              任务 →
            </Link>
            <span className={connected ? "live on" : "live off"}>
              {connected ? "实时已连接" : "实时连接中断，正在重连…"}
            </span>
          </>
        }
      />

      {error !== null && (
        <p className="banner error" role="alert">
          {error}
        </p>
      )}

      {listLoading ? (
        <p className="muted">加载运行记录…</p>
      ) : (
        <RunDetailPanel
          activeRuns={activeRuns}
          recentRuns={runs}
          selectedRunId={selectedRunId}
          detail={detail?.run.id === selectedRunId ? detail : null}
          activities={activities}
          messages={messages}
          loading={detailLoading}
          error={detailError}
          now={now}
          agentById={agentById}
          onSelect={(nextRunId) =>
            navigate(`/rooms/${roomId ?? ""}/runs/${nextRunId}`)
          }
        />
      )}
    </section>
  );
}
