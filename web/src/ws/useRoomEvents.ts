import { useEffect, useRef, useState } from "react";
import type { RoomEvent } from "@moon/shared";
import { websocketUrl } from "../runtime";

/** 首次重连延迟；连续失败按指数退避，上限 `MAX_RETRY_DELAY_MS`。 */
const INITIAL_RETRY_DELAY_MS = 500;
const MAX_RETRY_DELAY_MS = 5000;

export interface RoomEventsConnection {
  /** WebSocket 当前是否已连接（用于展示实时状态）。 */
  connected: boolean;
}

export function isRoomEvent(value: unknown): value is RoomEvent {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  const type = (value as { type?: unknown }).type;
  return (
    type === "run_started" ||
    type === "message_started" ||
    type === "message_delta" ||
    type === "message_final" ||
    type === "message_reactions_changed" ||
    type === "task_changed" ||
    type === "discussion_changed" ||
    type === "collaboration_changed" ||
    type === "run_activity" ||
    type === "history_cleared"
  );
}

/**
 * 订阅单个房间的实时事件。
 *
 * 唯一职责：建立 WS 连接、解析事件、把事件交给 `onEvent`、断线退避重连，
 * 并在房间变化/组件卸载时清理连接与定时器。不做缓存、不做业务状态合并。
 */
export function useRoomEvents(
  roomId: number | null,
  onEvent: (event: RoomEvent) => void,
): RoomEventsConnection {
  const [connected, setConnected] = useState(false);
  const handlerRef = useRef(onEvent);

  // 回调可能每次渲染都是新函数；用 ref 承接，避免因此重建连接。
  useEffect(() => {
    handlerRef.current = onEvent;
  }, [onEvent]);

  useEffect(() => {
    if (roomId === null || !Number.isSafeInteger(roomId) || roomId <= 0) {
      setConnected(false);
      return;
    }

    let disposed = false;
    let socket: WebSocket | null = null;
    let retryTimer: ReturnType<typeof setTimeout> | null = null;
    let attempt = 0;

    const connect = (): void => {
      if (disposed) {
        return;
      }

      const url = websocketUrl(`/ws?roomId=${roomId}`);
      const current = new WebSocket(url);
      socket = current;

      current.onopen = () => {
        if (disposed) {
          current.close();
          return;
        }
        attempt = 0;
        setConnected(true);
      };

      current.onmessage = (message) => {
        let parsed: unknown;
        try {
          parsed = JSON.parse(String(message.data));
        } catch {
          // 非 JSON 帧直接忽略，不影响后续事件。
          return;
        }
        if (isRoomEvent(parsed)) {
          handlerRef.current(parsed);
        }
      };

      current.onerror = () => {
        // 统一由 onclose 触发重连，避免错误与关闭各排一次定时器。
      };

      current.onclose = () => {
        if (disposed) {
          return;
        }
        setConnected(false);
        const delay = Math.min(
          INITIAL_RETRY_DELAY_MS * 2 ** attempt,
          MAX_RETRY_DELAY_MS,
        );
        attempt += 1;
        retryTimer = setTimeout(connect, delay);
      };
    };

    connect();

    return () => {
      disposed = true;
      if (retryTimer !== null) {
        clearTimeout(retryTimer);
        retryTimer = null;
      }
      socket?.close();
      socket = null;
      setConnected(false);
    };
  }, [roomId]);

  return { connected };
}
