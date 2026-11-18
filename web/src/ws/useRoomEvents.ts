import { useEffect, useRef, useState } from "react";
import type {
  ResyncRequiredFrame,
  RoomEvent,
  RoomEventFrame,
  RoomWatermark,
} from "@moon/shared";
import { websocketUrl } from "../runtime";

// 帧形状与水位类型由 `@moon/shared` 单一契约源提供，这里只做再导出，
// 让既有 import 位置保持可用。
export type { ResyncRequiredFrame, RoomEventFrame, RoomWatermark } from "@moon/shared";

/** 首次重连延迟；连续失败按指数退避，上限 `MAX_RETRY_DELAY_MS`。 */
const INITIAL_RETRY_DELAY_MS = 500;
const MAX_RETRY_DELAY_MS = 5000;

export interface RoomEventsConnection {
  /** WebSocket 当前是否已连接（用于展示实时状态）。 */
  connected: boolean;
  /** 服务端要求重同步时自增；消费方应据此重拉权威快照。 */
  resyncToken: number;
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
    type === "state_changed" ||
    type === "run_activity" ||
    type === "history_cleared"
  );
}

/** 帧校验：领域事件必须带安全整数 `epoch`/`seq`，缺失或损坏的帧直接丢弃。 */
export function isRoomEventFrame(value: unknown): value is RoomEventFrame {
  if (!isRoomEvent(value)) {
    return false;
  }
  const frame = value as { epoch?: unknown; seq?: unknown };
  return (
    isSafeSeq(frame.epoch) && isSafeSeq(frame.seq)
  );
}

/** 重同步信号校验：只关心类型与房间，避免把损坏帧当成领域事件。 */
export function isResyncRequiredFrame(
  value: unknown,
): value is ResyncRequiredFrame {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  const candidate = value as { type?: unknown; roomId?: unknown };
  return (
    candidate.type === "resync_required" &&
    typeof candidate.roomId === "number" &&
    Number.isSafeInteger(candidate.roomId)
  );
}

/** 连接路径：已知水位时带回 `epoch` 与 `seq`，让服务端判断补发还是重同步。 */
export function roomSocketPath(
  roomId: number,
  watermark: RoomWatermark | null,
): string {
  return watermark === null
    ? `/ws?roomId=${roomId}`
    : `/ws?roomId=${roomId}&afterEpoch=${watermark.epoch}&afterSeq=${watermark.seq}`;
}

/** 帧是否来自另一个服务端进程；此时本地水位不可比，必须重同步。 */
export function shouldResyncOnFrame(
  frame: Pick<RoomEventFrame, "epoch" | "seq">,
  watermark: RoomWatermark | null,
): boolean {
  return watermark !== null && frame.epoch !== watermark.epoch;
}

/**
 * 帧是否已处理过。
 *
 * 同一 epoch 内补发以水位为界，重复帧只可能来自异常重放；
 * 直接丢弃可避免 `message_delta` 被重复拼接。
 */
export function isDuplicateFrame(
  frame: Pick<RoomEventFrame, "epoch" | "seq">,
  watermark: RoomWatermark | null,
): boolean {
  return (
    watermark !== null &&
    frame.epoch === watermark.epoch &&
    frame.seq <= watermark.seq
  );
}

function isSafeSeq(value: unknown): value is number {
  return (
    typeof value === "number" && Number.isSafeInteger(value) && value >= 0
  );
}

/**
 * 订阅单个房间的实时事件。
 *
 * 唯一职责：建立 WS 连接、校验帧、维护 `epoch`/`seq` 水位并在断线后退避重连；
 * 服务端确认无法补齐时自增 `resyncToken`，由消费方重拉权威快照。
 * 不做缓存、不做业务状态合并。
 */
export function useRoomEvents(
  roomId: number | null,
  onEvent: (event: RoomEventFrame) => void,
): RoomEventsConnection {
  const [connected, setConnected] = useState(false);
  const [resyncToken, setResyncToken] = useState(0);
  const handlerRef = useRef(onEvent);
  // 已处理的最大水位；null 表示没有可用水位（首次连接或已被要求重同步）。
  const watermarkRef = useRef<RoomWatermark | null>(null);

  // 回调可能每次渲染都是新函数；用 ref 承接，避免因此重建连接。
  useEffect(() => {
    handlerRef.current = onEvent;
  }, [onEvent]);

  useEffect(() => {
    // 切房后水位失效：旧房间的 seq 不能用于新房间的补发。
    watermarkRef.current = null;

    if (roomId === null || !Number.isSafeInteger(roomId) || roomId <= 0) {
      setConnected(false);
      return;
    }

    let disposed = false;
    let socket: WebSocket | null = null;
    let retryTimer: ReturnType<typeof setTimeout> | null = null;
    let attempt = 0;

    const requestResync = (): void => {
      watermarkRef.current = null;
      setResyncToken((token) => token + 1);
    };

    const connect = (): void => {
      if (disposed) {
        return;
      }

      const current = new WebSocket(
        websocketUrl(roomSocketPath(roomId, watermarkRef.current)),
      );
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
        // 旧连接在 cleanup 之后仍可能排队投递帧，不能污染新房间的水位。
        if (disposed || socket !== current) {
          return;
        }

        let parsed: unknown;
        try {
          parsed = JSON.parse(String(message.data));
        } catch {
          // 非 JSON 帧直接忽略，不影响后续事件。
          return;
        }

        if (isResyncRequiredFrame(parsed)) {
          if (parsed.roomId !== roomId) {
            return;
          }
          requestResync();
          return;
        }

        if (!isRoomEventFrame(parsed)) {
          return;
        }

        if (shouldResyncOnFrame(parsed, watermarkRef.current)) {
          // 服务端进程已更换：本连接上的序号不可比，重拉快照后再继续。
          requestResync();
          return;
        }

        if (isDuplicateFrame(parsed, watermarkRef.current)) {
          return;
        }

        watermarkRef.current = { epoch: parsed.epoch, seq: parsed.seq };
        handlerRef.current(parsed);
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

  return { connected, resyncToken };
}
