import type {
  Message,
  MessageStatus,
  RoomEvent,
  RoomState,
} from "@moon/shared";

/** 进行中的运行：运行状态条与耗时展示用。 */
export interface ActiveRun {
  runId: number;
  agentId: number;
  messageId: number;
  startedAt: number;
}

/** 房间聊天状态：消息、运行登记与未读提示，全部由事件驱动。 */
export interface RoomChatState {
  messages: Message[];
  activeRuns: Record<number, ActiveRun>;
  /** 已结束运行的耗时（毫秒），按 runId 记录。 */
  runDurations: Record<number, number>;
  /** 消息 id → 运行 id；阶段消息没有 runId 时用它回溯所属运行。 */
  runIdByMessageId: Record<number, number>;
  /** 屏幕阅读器播报文案。 */
  announcement: string;
  unseenCount: number;
}

/**
 * 事件处理的副作用。
 *
 * reducer 只产出数据，由 `useRoomChat` 统一应用（刷新键、重拉、页面回调），
 * 保持 reducer 纯净、可单测。
 */
export type RoomChatEffect =
  | { kind: "refresh"; target: "context" | "queue" | "files" }
  | { kind: "reload_messages" }
  | { kind: "state"; state: RoomState }
  | { kind: "resync" };

export type RoomChatAction =
  | { kind: "reset" }
  /** 已看到最新消息：未读数清零。 */
  | { kind: "mark_seen" }
  /** 与已有消息合并（历史加载、重拉）。 */
  | { kind: "history"; messages: Message[] }
  /** 整体替换（历史被清理后重拉）。 */
  | { kind: "replace_history"; messages: Message[] }
  /** 重同步：`messages` 为服务端快照，拉取失败时传 null。 */
  | { kind: "resync"; messages: Message[] | null; now: number }
  | {
      kind: "event";
      event: RoomEvent;
      /** 当前房间；跨房事件直接丢弃。 */
      roomId: number;
      /** 收到事件时列表是否贴底；不贴底才累积未读数。 */
      atBottom: boolean;
      now: number;
    };

export interface RoomChatResult {
  state: RoomChatState;
  effects: RoomChatEffect[];
}

export const initialRoomChatState: RoomChatState = {
  messages: [],
  activeRuns: {},
  runDurations: {},
  runIdByMessageId: {},
  announcement: "",
  unseenCount: 0,
};

/** 消息状态优先级：终态（final/failed）高于流式中的占位消息。 */
const MESSAGE_STATUS_PRIORITY: Record<MessageStatus, number> = {
  streaming: 0,
  final: 1,
  failed: 1,
};

/**
 * 同一 id 的两份快照取“更新”的一份：终态优先于 streaming，
 * 同状态取内容更长的一份（流式增量只会增长）。
 */
function pickNewerMessage(left: Message, right: Message): Message {
  const leftPriority = MESSAGE_STATUS_PRIORITY[left.status];
  const rightPriority = MESSAGE_STATUS_PRIORITY[right.status];
  if (leftPriority !== rightPriority) {
    return rightPriority > leftPriority ? right : left;
  }
  return right.content.length >= left.content.length ? right : left;
}

/**
 * 合并两份消息快照：按 id 去重、按 seq 升序。
 *
 * 历史加载完成时用它与已到达的实时消息合并，而不是直接覆盖状态；
 * 同一 id 的优先级规则见 `pickNewerMessage`。
 */
export function mergeMessages(
  current: readonly Message[],
  incoming: readonly Message[],
): Message[] {
  const byId = new Map<number, Message>();
  for (const message of current) {
    byId.set(message.id, message);
  }
  for (const message of incoming) {
    const existing = byId.get(message.id);
    byId.set(
      message.id,
      existing === undefined ? message : pickNewerMessage(existing, message),
    );
  }
  return [...byId.values()].sort((a, b) => a.seq - b.seq);
}

/**
 * 事件是否属于当前房间。契约要求所有事件都带 `roomId`，
 * 切换房间后旧连接可能仍投递事件，必须在此丢弃。
 */
export function isEventForRoom(
  event: RoomEvent,
  roomId: number | null,
): boolean {
  return roomId !== null && event.roomId === roomId;
}

/**
 * 从落库消息重建"仍在运行"的登记。
 *
 * 只有 `streaming` 消息代表运行未结束；断线期间完成的运行已经没有
 * `message_final` 可达，重同步后必须按服务端事实清掉，避免幽灵运行。
 */
export function collectStreamingRuns(
  messages: readonly Message[],
  now: number,
): {
  runs: Record<number, ActiveRun>;
  byMessage: Record<number, number>;
} {
  const runs: Record<number, ActiveRun> = {};
  const byMessage: Record<number, number> = {};
  for (const message of messages) {
    if (
      message.status !== "streaming" ||
      message.runId === null ||
      message.senderAgentId === null
    ) {
      continue;
    }
    runs[message.runId] = {
      runId: message.runId,
      agentId: message.senderAgentId,
      messageId: message.id,
      startedAt: now,
    };
    byMessage[message.id] = message.runId;
  }
  return { runs, byMessage };
}

const REFRESH_ALL: RoomChatEffect[] = [
  { kind: "refresh", target: "context" },
  { kind: "refresh", target: "queue" },
  { kind: "refresh", target: "files" },
];

/** 房间聊天状态机：把实时事件与历史快照折算成新状态和副作用。 */
export function roomChatReducer(
  state: RoomChatState,
  action: RoomChatAction,
): RoomChatResult {
  switch (action.kind) {
    case "reset":
      return { state: initialRoomChatState, effects: [] };
    case "history":
      return {
        state: { ...state, messages: mergeMessages(state.messages, action.messages) },
        effects: [],
      };
    case "replace_history":
      return { state: { ...state, messages: action.messages }, effects: [] };
    case "mark_seen":
      return state.unseenCount === 0
        ? { state, effects: [] }
        : { state: { ...state, unseenCount: 0 }, effects: [] };
    case "resync":
      return applyResync(state, action.messages, action.now);
    case "event":
      return applyEvent(state, action);
  }
}

function applyResync(
  state: RoomChatState,
  messages: Message[] | null,
  now: number,
): RoomChatResult {
  // 页面级重拉即使消息拉取失败也必须执行：讨论/委派可能同样断线过。
  const effects: RoomChatEffect[] = [...REFRESH_ALL, { kind: "resync" }];
  if (messages === null) {
    return { state, effects };
  }
  const merged = mergeMessages(state.messages, messages);
  const { runs, byMessage } = collectStreamingRuns(merged, now);
  return {
    state: {
      ...state,
      messages: merged,
      activeRuns: runs,
      runIdByMessageId: byMessage,
    },
    effects,
  };
}

function applyEvent(
  state: RoomChatState,
  action: Extract<RoomChatAction, { kind: "event" }>,
): RoomChatResult {
  const { event, roomId, atBottom, now } = action;

  // 丢弃跨房间的过期事件：它们不属于当前页面展示的消息列表。
  if (!isEventForRoom(event, roomId)) {
    return { state, effects: [] };
  }

  // 状态快照由页面持有；聊天面板只上抛，不自留副本。
  if (event.type === "state_changed") {
    return { state, effects: [{ kind: "state", state: event.state }] };
  }

  // 运行活动由独立运行详情页消费，聊天面板不保留活动状态。
  if (event.type === "run_activity") {
    return { state, effects: [] };
  }

  // 纯聊天历史已清理（可能来自其它标签页），整体重拉列表。
  if (event.type === "history_cleared") {
    return {
      state,
      effects: [...REFRESH_ALL, { kind: "reload_messages" }],
    };
  }

  // 反应只替换目标消息的 reactions，不动正文与运行状态。
  if (event.type === "message_reactions_changed") {
    return {
      state: {
        ...state,
        messages: state.messages.map((item) =>
          item.id === event.messageId
            ? { ...item, reactions: event.reactions }
            : item,
        ),
      },
      effects: [],
    };
  }

  if (event.type === "run_started") {
    return {
      state: {
        ...state,
        messages: mergeMessages(state.messages, [event.message]),
        activeRuns: {
          ...state.activeRuns,
          [event.runId]: {
            runId: event.runId,
            agentId: event.agentId,
            messageId: event.messageId,
            startedAt: now,
          },
        },
        runIdByMessageId: {
          ...state.runIdByMessageId,
          [event.messageId]: event.runId,
        },
        announcement: "成员开始回复",
      },
      effects: [{ kind: "refresh", target: "queue" }],
    };
  }

  // 同一运行进入新阶段：登记新消息，运行状态保持不变。
  if (event.type === "message_started") {
    return {
      state: {
        ...state,
        messages: mergeMessages(state.messages, [event.message]),
        runIdByMessageId: {
          ...state.runIdByMessageId,
          [event.message.id]: event.runId,
        },
      },
      effects: [],
    };
  }

  if (event.type === "message_delta") {
    return {
      state: {
        ...state,
        messages: state.messages.map((item) =>
          item.id === event.messageId
            ? { ...item, content: item.content + event.text }
            : item,
        ),
      },
      effects: [],
    };
  }

  // message_final：progress 只落定阶段消息；reply 才结束整个运行。
  const finishedRunId =
    event.message.runId ?? state.runIdByMessageId[event.message.id] ?? null;
  const finishedRun =
    finishedRunId === null ? undefined : state.activeRuns[finishedRunId];

  let next = state;
  if (finishedRun !== undefined && event.message.kind === "reply") {
    const activeRuns = { ...state.activeRuns };
    delete activeRuns[finishedRunId!];
    next = {
      ...state,
      activeRuns,
      runDurations: {
        ...state.runDurations,
        [finishedRunId!]: now - finishedRun.startedAt,
      },
    };
  }

  if (event.message.kind === "reply") {
    // 用户/系统/成员的任何 final reply 都会进入历史，统一刷新估算。
    if (event.message.senderType === "agent") {
      next = {
        ...next,
        announcement: "成员回复已完成",
        unseenCount: atBottom ? next.unseenCount : next.unseenCount + 1,
      };
    }
    return {
      state: {
        ...next,
        messages: mergeMessages(next.messages, [event.message]),
      },
      effects: [...REFRESH_ALL],
    };
  }

  return {
    state: {
      ...next,
      messages: mergeMessages(next.messages, [event.message]),
    },
    effects: [],
  };
}
