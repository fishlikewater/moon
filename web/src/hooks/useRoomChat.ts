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
import type { Message, RoomEvent, RoomState } from "@moon/shared";
import { api, errorMessage } from "../api/client";
import { useRoomEvents } from "../ws/useRoomEvents";
import {
  initialRoomChatState,
  roomChatReducer,
  type ActiveRun,
  type RoomChatAction,
  type RoomChatEffect,
  type RoomChatState,
} from "./roomChatReducer";

// 既有导出位置保持不变：RoomPage 从这里 re-export，页面与测试都依赖它。
export { collectStreamingRuns, isEventForRoom, mergeMessages } from "./roomChatReducer";
export type { ActiveRun } from "./roomChatReducer";

export interface RoomChatOptions {
  /** 房间状态快照；状态由页面持有，hook 只上抛，合并规则由页面决定。 */
  onState?: (state: RoomState) => void;
  /** 实时链路要求重同步时回调；页面据此重拉讨论/委派等 REST 快照。 */
  onResync?: () => void;
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
 * 房间聊天状态机：历史消息加载、实时事件分派、运行登记与滚动贴底。
 *
 * 状态变更全部交给纯函数 `roomChatReducer`，副作用以数据返回后在此统一应用；
 * RoomPage 与直聊页共用本 hook，讨论/委派快照通过 `onState` 上抛给页面。
 */
export function useRoomChat(
  roomId: number | null,
  options: RoomChatOptions = {},
): RoomChat {
  const [state, setState] = useState<RoomChatState>(initialRoomChatState);
  // 事件在同一 tick 内连续到达时必须读到最新状态，不能依赖渲染后的 state。
  const stateRef = useRef<RoomChatState>(initialRoomChatState);
  const [stickToBottom, setStickToBottom] = useState(true);
  const [contextRefreshKey, setContextRefreshKey] = useState(0);
  const [queueRefreshKey, setQueueRefreshKey] = useState(0);
  const [filesRefreshKey, setFilesRefreshKey] = useState(0);
  const [historyLoading, setHistoryLoading] = useState(roomId !== null);

  const messagesRef = useRef<Message[]>(initialRoomChatState.messages);
  const stickRef = useRef(true);
  const listRef = useRef<HTMLDivElement | null>(null);
  const optionsRef = useRef(options);
  const reloadMessagesRef = useRef<() => Promise<void>>(async () => {});

  // 回调可能每次渲染都是新函数；用 ref 承接，避免事件处理随之重建。
  useEffect(() => {
    optionsRef.current = options;
  });

  useEffect(() => {
    messagesRef.current = state.messages;
  }, [state.messages]);

  const applyEffect = useCallback((effect: RoomChatEffect) => {
    switch (effect.kind) {
      case "refresh":
        if (effect.target === "context") {
          setContextRefreshKey((key) => key + 1);
        } else if (effect.target === "queue") {
          setQueueRefreshKey((key) => key + 1);
        } else {
          setFilesRefreshKey((key) => key + 1);
        }
        return;
      case "reload_messages":
        void reloadMessagesRef.current();
        return;
      case "state":
        optionsRef.current.onState?.(effect.state);
        return;
      case "resync":
        optionsRef.current.onResync?.();
        return;
    }
  }, []);

  /** 唯一的状态写入口：纯 reducer 计算后同步写入，再应用副作用。 */
  const dispatchAction = useCallback(
    (action: RoomChatAction) => {
      const result = roomChatReducer(stateRef.current, action);
      stateRef.current = result.state;
      setState(result.state);
      for (const effect of result.effects) {
        applyEffect(effect);
      }
    },
    [applyEffect],
  );

  /** 历史清理后整体重拉；清理期间房间被判定为空闲，不存在流式消息。 */
  const reloadMessages = useCallback(async () => {
    if (roomId === null) {
      return;
    }
    try {
      const messages = await api.messages.list(roomId);
      dispatchAction({ kind: "replace_history", messages });
    } catch (cause) {
      optionsRef.current.onError?.(errorMessage(cause));
    }
  }, [roomId, dispatchAction]);

  // effect 里通过 ref 触发重拉，避免 applyEffect 与 reloadMessages 互相依赖。
  useEffect(() => {
    reloadMessagesRef.current = reloadMessages;
  }, [reloadMessages]);

  /** 页面侧的整表替换/补丁：读到最新消息表后写回。 */
  const setMessages = useCallback<Dispatch<SetStateAction<Message[]>>>((value) => {
    const current = stateRef.current;
    const next =
      typeof value === "function"
        ? (value as (previous: Message[]) => Message[])(current.messages)
        : value;
    if (next === current.messages) {
      return;
    }
    const updated = { ...current, messages: next };
    stateRef.current = updated;
    setState(updated);
  }, []);

  /** 按 id 合并：已存在则按状态优先级取更新的一份，否则按 seq 插入。 */
  const mergeMessage = useCallback(
    (incoming: Message) => {
      dispatchAction({ kind: "history", messages: [incoming] });
    },
    [dispatchAction],
  );

  const handleEvent = useCallback(
    (event: RoomEvent) => {
      if (roomId === null) {
        return;
      }
      dispatchAction({
        kind: "event",
        event,
        roomId,
        atBottom: stickRef.current,
        now: Date.now(),
      });
    },
    [dispatchAction, roomId],
  );

  const { connected, resyncToken } = useRoomEvents(roomId, handleEvent);

  // 服务端确认无法按 seq 补齐（或判定服务端序号已重置）：以服务端快照为准，
  // 合并消息并按 `streaming` 状态重建运行登记，再让页面重拉讨论/委派。
  const resyncSeenRef = useRef(0);
  useEffect(() => {
    if (resyncToken === resyncSeenRef.current) {
      return;
    }
    resyncSeenRef.current = resyncToken;
    if (roomId === null) {
      return;
    }

    let cancelled = false;
    void (async () => {
      let messages: Message[] | null = null;
      try {
        messages = await api.messages.list(roomId);
      } catch (cause) {
        if (!cancelled) {
          optionsRef.current.onError?.(errorMessage(cause));
        }
      }
      if (cancelled) {
        return;
      }
      // 拉取失败也要派发：页面级重拉不依赖消息请求是否成功。
      dispatchAction({ kind: "resync", messages, now: Date.now() });
    })();

    return () => {
      cancelled = true;
    };
  }, [resyncToken, roomId, dispatchAction]);

  // 切房先清空聊天状态机，避免上一个房间的消息/运行混入。
  useEffect(() => {
    dispatchAction({ kind: "reset" });
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
        dispatchAction({ kind: "history", messages: history });
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
  }, [roomId, dispatchAction]);

  const runningCount = Object.keys(state.activeRuns).length;

  const scrollToBottom = useCallback((behavior: ScrollBehavior = "auto") => {
    const list = listRef.current;
    if (list === null) {
      return;
    }
    if (behavior === "smooth") {
      list.scrollTo({ top: list.scrollHeight, behavior });
      return;
    }
    list.scrollTop = list.scrollHeight;
  }, []);

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
  }, [state.messages, stickToBottom, scrollToBottom]);

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
      dispatchAction({ kind: "mark_seen" });
    }
  }, [dispatchAction]);

  const jumpToLatest = useCallback(() => {
    stickRef.current = true;
    setStickToBottom(true);
    dispatchAction({ kind: "mark_seen" });
    scrollToBottom("smooth");
  }, [dispatchAction, scrollToBottom]);

  const onMessageLayout = useCallback(() => {
    if (!stickRef.current) {
      return;
    }
    scrollToBottom();
  }, [scrollToBottom]);

  const resetStick = useCallback(() => {
    stickRef.current = true;
    setStickToBottom(true);
    dispatchAction({ kind: "mark_seen" });
    scrollToBottom();
  }, [dispatchAction, scrollToBottom]);

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
    messages: state.messages,
    setMessages,
    messagesRef,
    activeRuns: state.activeRuns,
    runDurations: state.runDurations,
    runIdByMessageId: state.runIdByMessageId,
    announcement: state.announcement,
    unseenCount: state.unseenCount,
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
