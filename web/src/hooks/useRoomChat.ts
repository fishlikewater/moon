import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type Dispatch,
  type RefObject,
  type SetStateAction,
} from "react";
import type {
  CollaborationBatch,
  Discussion,
  Message,
  MessageStatus,
  RoomEvent,
} from "@moon/shared";
import { api, errorMessage } from "../api/client";
import { useRoomEvents } from "../ws/useRoomEvents";

/** 进行中的运行：运行状态条与耗时展示用。 */
export interface ActiveRun {
  runId: number;
  agentId: number;
  messageId: number;
  startedAt: number;
}

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

export interface RoomChatOptions {
  /** 讨论快照；讨论状态由页面持有，hook 只上抛。 */
  onDiscussion?: (discussion: Discussion) => void;
  /** 委派快照；页面自行按 `updatedAt` 丢弃迟到的旧广播。 */
  onCollaboration?: (batch: CollaborationBatch) => void;
  /** 历史消息加载或重拉失败时上报，由页面决定错误条展示。 */
  onError?: (message: string) => void;
}

export interface RoomChat {
  messages: Message[];
  setMessages: Dispatch<SetStateAction<Message[]>>;
  /** 事件处理里需要读最新消息（如表情回滚），避免额外依赖触发重建。 */
  messagesRef: RefObject<Message[]>;
  activeRuns: Record<number, ActiveRun>;
  runDurations: Record<number, number>;
  runIdByMessageId: Record<number, number>;
  announcement: string;
  unseenCount: number;
  stickToBottom: boolean;
  runningCount: number;
  contextRefreshKey: number;
  queueRefreshKey: number;
  filesRefreshKey: number;
  /** 首次历史快照尚未落地时为 true，避免空态先闪一下。 */
  historyLoading: boolean;
  connected: boolean;
  /** 页面主动触发上下文用量/队列/文件刷新（发送消息、取消队列任务等）。 */
  refreshContext: () => void;
  refreshQueue: () => void;
  refreshFiles: () => void;
  /** 消息滚动容器；贴底判定与自动跟随都基于它。 */
  listRef: RefObject<HTMLDivElement | null>;
  onListScroll: () => void;
  onMessageLayout: () => void;
  jumpToLatest: () => void;
  /** 筛选变化后回到最新并清零未读数。 */
  resetStick: () => void;
  mergeMessage: (incoming: Message) => void;
  reloadMessages: () => Promise<void>;
}

/**
 * 房间聊天状态机：历史消息加载/合并、实时事件分派、运行登记与滚动贴底。
 *
 * RoomPage 与直聊页共用；讨论/委派等房间专属状态通过 options 回调上抛，
 * 由页面持有，hook 不感知房间布局与侧栏。
 */
export function useRoomChat(
  roomId: number | null,
  options: RoomChatOptions = {},
): RoomChat {
  const [messages, setMessages] = useState<Message[]>([]);
  const [activeRuns, setActiveRuns] = useState<Record<number, ActiveRun>>({});
  const [runDurations, setRunDurations] = useState<Record<number, number>>({});
  const [runIdByMessageId, setRunIdByMessageId] = useState<
    Record<number, number>
  >({});
  const [announcement, setAnnouncement] = useState("");
  const [unseenCount, setUnseenCount] = useState(0);
  const [stickToBottom, setStickToBottom] = useState(true);
  const [contextRefreshKey, setContextRefreshKey] = useState(0);
  const [queueRefreshKey, setQueueRefreshKey] = useState(0);
  const [filesRefreshKey, setFilesRefreshKey] = useState(0);
  const [historyLoading, setHistoryLoading] = useState(roomId !== null);

  const messagesRef = useRef<Message[]>([]);
  const activeRunsRef = useRef<Record<number, ActiveRun>>({});
  const runIdByMessageIdRef = useRef<Record<number, number>>({});
  const listRef = useRef<HTMLDivElement | null>(null);
  const stickRef = useRef(true);
  const optionsRef = useRef(options);

  // 回调可能每次渲染都是新函数；用 ref 承接，避免事件处理随之重建。
  useEffect(() => {
    optionsRef.current = options;
  });

  useEffect(() => {
    messagesRef.current = messages;
  }, [messages]);

  // 切房先清空聊天状态机，避免上一个房间的消息/运行混入。
  useEffect(() => {
    setMessages([]);
    setActiveRuns({});
    activeRunsRef.current = {};
    setRunDurations({});
    setRunIdByMessageId({});
    runIdByMessageIdRef.current = {};
    setUnseenCount(0);
    setAnnouncement("");
    stickRef.current = true;
    setStickToBottom(true);

    if (roomId === null) {
      setHistoryLoading(false);
      return;
    }

    let cancelled = false;
    setHistoryLoading(true);
    void (async () => {
      try {
        const history = await api.messages.list(roomId);
        if (cancelled) {
          return;
        }
        // 历史快照与已到达的实时消息合并，避免覆盖流式增量/终态。
        setMessages((previous) => mergeMessages(previous, history));
      } catch (cause) {
        if (!cancelled) {
          optionsRef.current.onError?.(errorMessage(cause));
        }
      } finally {
        if (!cancelled) {
          setHistoryLoading(false);
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [roomId]);

  /** 按 id 合并：已存在则按状态优先级取更新的一份，否则按 seq 插入。 */
  const mergeMessage = useCallback((incoming: Message) => {
    setMessages((previous) => mergeMessages(previous, [incoming]));
  }, []);

  /** 历史清理后整体重拉；清理期间房间被判定为空闲，不存在流式消息。 */
  const reloadMessages = useCallback(async () => {
    if (roomId === null) {
      return;
    }
    try {
      setMessages(await api.messages.list(roomId));
    } catch (cause) {
      optionsRef.current.onError?.(errorMessage(cause));
    }
  }, [roomId]);

  const handleEvent = useCallback(
    (event: RoomEvent) => {
      // 丢弃跨房间的过期事件：它们不属于当前页面展示的消息列表。
      if (!isEventForRoom(event, roomId)) {
        return;
      }

      // 任务状态变化由任务页展示，聊天面板不消费该事件。
      if (event.type === "task_changed") {
        return;
      }

      // 讨论状态条只消费快照；讨论消息本身仍走 message_final。
      if (event.type === "discussion_changed") {
        optionsRef.current.onDiscussion?.(event.discussion);
        return;
      }

      // 委派批次同样只消费快照；快照合并规则由页面决定。
      if (event.type === "collaboration_changed") {
        optionsRef.current.onCollaboration?.(event.batch);
        return;
      }

      // 运行活动由独立运行详情页消费，聊天面板不保留活动状态。
      if (event.type === "run_activity") {
        return;
      }

      // 纯聊天历史已清理（可能来自其它标签页），整体重拉列表。
      if (event.type === "history_cleared") {
        setContextRefreshKey((key) => key + 1);
        setQueueRefreshKey((key) => key + 1);
        setFilesRefreshKey((key) => key + 1);
        void reloadMessages();
        return;
      }

      // 反应只替换目标消息的 reactions，不动正文与运行状态。
      if (event.type === "message_reactions_changed") {
        setMessages((previous) =>
          previous.map((item) =>
            item.id === event.messageId
              ? { ...item, reactions: event.reactions }
              : item,
          ),
        );
        return;
      }

      if (event.type === "run_started") {
        const run: ActiveRun = {
          runId: event.runId,
          agentId: event.agentId,
          messageId: event.messageId,
          startedAt: Date.now(),
        };
        activeRunsRef.current = {
          ...activeRunsRef.current,
          [event.runId]: run,
        };
        setActiveRuns((previous) => ({
          ...previous,
          [event.runId]: run,
        }));
        setRunIdByMessageId((previous) => ({
          ...previous,
          [event.messageId]: event.runId,
        }));
        runIdByMessageIdRef.current = {
          ...runIdByMessageIdRef.current,
          [event.messageId]: event.runId,
        };
        setAnnouncement("成员开始回复");
        setQueueRefreshKey((key) => key + 1);
        mergeMessage(event.message);
        return;
      }

      // 同一运行进入新阶段：登记新消息，运行状态保持不变。
      if (event.type === "message_started") {
        setRunIdByMessageId((previous) => ({
          ...previous,
          [event.message.id]: event.runId,
        }));
        runIdByMessageIdRef.current = {
          ...runIdByMessageIdRef.current,
          [event.message.id]: event.runId,
        };
        mergeMessage(event.message);
        return;
      }

      if (event.type === "message_delta") {
        setMessages((previous) =>
          previous.map((item) =>
            item.id === event.messageId
              ? { ...item, content: item.content + event.text }
              : item,
          ),
        );
        return;
      }

      // message_final：progress 只落定阶段消息；reply 才结束整个运行。
      const finishedRunId =
        event.message.runId ??
        runIdByMessageIdRef.current[event.message.id] ??
        null;
      const finishedRun =
        finishedRunId === null ? undefined : activeRunsRef.current[finishedRunId];
      if (finishedRun !== undefined && event.message.kind === "reply") {
        const nextRuns = { ...activeRunsRef.current };
        delete nextRuns[finishedRunId!];
        activeRunsRef.current = nextRuns;
        setRunDurations((previous) => ({
          ...previous,
          [finishedRunId!]: Date.now() - finishedRun.startedAt,
        }));
        setActiveRuns((previous) => {
          if (!(finishedRunId! in previous)) {
            return previous;
          }
          const next = { ...previous };
          delete next[finishedRunId!];
          return next;
        });
      }
      if (event.message.kind === "reply") {
        // 用户/系统/成员的任何 final reply 都会进入历史，统一刷新估算。
        setContextRefreshKey((key) => key + 1);
        setQueueRefreshKey((key) => key + 1);
        setFilesRefreshKey((key) => key + 1);
        if (event.message.senderType === "agent") {
          setAnnouncement("成员回复已完成");
          if (!stickRef.current) {
            setUnseenCount((count) => count + 1);
          }
        }
      }
      mergeMessage(event.message);
    },
    [mergeMessage, reloadMessages, roomId],
  );

  const { connected } = useRoomEvents(roomId, handleEvent);

  const runningCount = Object.keys(activeRuns).length;

  const scrollToBottom = useCallback(
    (behavior: ScrollBehavior = "auto") => {
      const list = listRef.current;
      if (list === null) {
        return;
      }
      if (behavior === "smooth") {
        list.scrollTo({ top: list.scrollHeight, behavior });
        return;
      }
      list.scrollTop = list.scrollHeight;
    },
    [],
  );

  // Markdown chunk 加载完成后内容高度会变化；若仍贴底，再滚一次。
  useEffect(() => {
    let active = true;
    void import("../components/MarkdownContent").then(() => {
      if (!active || !stickRef.current) {
        return;
      }
      scrollToBottom();
    });
    return () => {
      active = false;
    };
  }, [scrollToBottom]);

  // messages 每次变化都代表内容高度可能变化；只有用户仍在底部时才跟随。
  useLayoutEffect(() => {
    if (!stickRef.current) {
      return;
    }
    scrollToBottom();
  }, [messages, stickToBottom, scrollToBottom]);

  // 运行状态条出现或消失会改变列表可视高度；仍在底部时重新贴底。
  useEffect(() => {
    if (!stickRef.current) {
      return;
    }
    scrollToBottom();
  }, [runningCount, scrollToBottom]);

  const onListScroll = useCallback(() => {
    const list = listRef.current;
    if (list === null) {
      return;
    }
    const distance = list.scrollHeight - list.scrollTop - list.clientHeight;
    const nextStick = distance < 80;
    stickRef.current = nextStick;
    setStickToBottom(nextStick);
    if (nextStick) {
      setUnseenCount(0);
    }
  }, []);

  const jumpToLatest = useCallback(() => {
    stickRef.current = true;
    setStickToBottom(true);
    setUnseenCount(0);
    scrollToBottom("smooth");
  }, [scrollToBottom]);

  const onMessageLayout = useCallback(() => {
    if (!stickRef.current) {
      return;
    }
    scrollToBottom();
  }, [scrollToBottom]);

  const resetStick = useCallback(() => {
    stickRef.current = true;
    setStickToBottom(true);
    setUnseenCount(0);
    scrollToBottom();
  }, [scrollToBottom]);

  const refreshContext = useCallback(() => {
    setContextRefreshKey((key) => key + 1);
  }, []);

  const refreshQueue = useCallback(() => {
    setQueueRefreshKey((key) => key + 1);
  }, []);

  const refreshFiles = useCallback(() => {
    setFilesRefreshKey((key) => key + 1);
  }, []);

  return {
    messages,
    setMessages,
    messagesRef,
    activeRuns,
    runDurations,
    runIdByMessageId,
    announcement,
    unseenCount,
    stickToBottom,
    runningCount,
    contextRefreshKey,
    queueRefreshKey,
    filesRefreshKey,
    historyLoading,
    connected,
    refreshContext,
    refreshQueue,
    refreshFiles,
    listRef,
    onListScroll,
    onMessageLayout,
    jumpToLatest,
    resetStick,
    mergeMessage,
    reloadMessages,
  };
}
