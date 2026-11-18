import { describe, expect, it } from "vitest";
import {
  isDuplicateFrame,
  isResyncRequiredFrame,
  isRoomEvent,
  isRoomEventFrame,
  roomSocketPath,
  shouldResyncOnFrame,
} from "./useRoomEvents";

/**
 * 契约守卫必须覆盖 shared 中全部 RoomEvent 类型。
 * 漏掉一种事件时，页面逻辑收不到该事件，实时状态会静默缺失。
 */
const EVENT_TYPES = [
  "run_started",
  "message_started",
  "message_delta",
  "message_final",
  "message_reactions_changed",
  "state_changed",
  "run_activity",
  "history_cleared",
] as const;

describe("isRoomEvent", () => {
  it("接受 shared 契约中的全部事件类型", () => {
    for (const type of EVENT_TYPES) {
      expect(isRoomEvent({ type }), type).toBe(true);
    }
  });

  it("拒绝未知类型与非对象值", () => {
    expect(isRoomEvent({ type: "unknown_event" })).toBe(false);
    expect(isRoomEvent({})).toBe(false);
    expect(isRoomEvent(null)).toBe(false);
    expect(isRoomEvent("run_started")).toBe(false);
  });

  it("拒绝已合并进 state_changed 的旧事件名", () => {
    expect(isRoomEvent({ type: "task_changed" })).toBe(false);
    expect(isRoomEvent({ type: "discussion_changed" })).toBe(false);
    expect(isRoomEvent({ type: "collaboration_changed" })).toBe(false);
  });
});

describe("isRoomEventFrame", () => {
  it("接受带安全整数 epoch/seq 的领域事件", () => {
    expect(
      isRoomEventFrame({ type: "message_delta", epoch: 11, seq: 1 }),
    ).toBe(true);
    expect(isRoomEventFrame({ type: "history_cleared", epoch: 0, seq: 0 })).toBe(
      true,
    );
  });

  it("拒绝缺少 epoch/seq 或取值非安全整数的帧", () => {
    expect(isRoomEventFrame({ type: "message_delta", seq: 1 })).toBe(false);
    expect(isRoomEventFrame({ type: "message_delta", epoch: 1 })).toBe(false);
    expect(
      isRoomEventFrame({ type: "message_delta", epoch: 1, seq: "1" }),
    ).toBe(false);
    expect(
      isRoomEventFrame({ type: "message_delta", epoch: 1, seq: 1.5 }),
    ).toBe(false);
    expect(
      isRoomEventFrame({ type: "message_delta", epoch: 1, seq: -1 }),
    ).toBe(false);
    expect(
      isRoomEventFrame({
        type: "message_delta",
        epoch: Number.MAX_SAFE_INTEGER + 1,
        seq: 1,
      }),
    ).toBe(false);
    expect(
      isRoomEventFrame({ type: "unknown_event", epoch: 1, seq: 1 }),
    ).toBe(false);
  });
});

describe("isResyncRequiredFrame", () => {
  it("只接受带合法 roomId 的重同步信号", () => {
    expect(isResyncRequiredFrame({ type: "resync_required", roomId: 3 })).toBe(
      true,
    );
    expect(isResyncRequiredFrame({ type: "resync_required" })).toBe(false);
    expect(
      isResyncRequiredFrame({ type: "resync_required", roomId: "3" }),
    ).toBe(false);
    expect(isResyncRequiredFrame({ type: "message_delta", roomId: 3 })).toBe(
      false,
    );
    expect(isResyncRequiredFrame(null)).toBe(false);
  });
});

describe("roomSocketPath", () => {
  it("首次连接不带水位", () => {
    expect(roomSocketPath(7, null)).toBe("/ws?roomId=7");
  });

  it("已知水位时同时带上 epoch 与 seq", () => {
    expect(roomSocketPath(7, { epoch: 99, seq: 0 })).toBe(
      "/ws?roomId=7&afterEpoch=99&afterSeq=0",
    );
    expect(roomSocketPath(7, { epoch: 99, seq: 42 })).toBe(
      "/ws?roomId=7&afterEpoch=99&afterSeq=42",
    );
  });
});

describe("shouldResyncOnFrame", () => {
  it("无水位时不做判定", () => {
    expect(shouldResyncOnFrame({ epoch: 1, seq: 1 }, null)).toBe(false);
  });

  it("epoch 不同说明服务端进程已更换", () => {
    expect(shouldResyncOnFrame({ epoch: 2, seq: 9 }, { epoch: 1, seq: 9 })).toBe(
      true,
    );
  });

  it("epoch 相同继续按 seq 判断", () => {
    expect(shouldResyncOnFrame({ epoch: 1, seq: 10 }, { epoch: 1, seq: 9 })).toBe(
      false,
    );
  });
});

describe("isDuplicateFrame", () => {
  it("无水位时不视为重复", () => {
    expect(isDuplicateFrame({ epoch: 1, seq: 1 }, null)).toBe(false);
  });

  it("同一 epoch 内不超过水位的帧视为重复", () => {
    expect(isDuplicateFrame({ epoch: 1, seq: 9 }, { epoch: 1, seq: 9 })).toBe(
      true,
    );
    expect(isDuplicateFrame({ epoch: 1, seq: 8 }, { epoch: 1, seq: 9 })).toBe(
      true,
    );
    expect(isDuplicateFrame({ epoch: 1, seq: 10 }, { epoch: 1, seq: 9 })).toBe(
      false,
    );
  });

  it("epoch 不同不属于重复，交由重同步判定", () => {
    expect(isDuplicateFrame({ epoch: 2, seq: 9 }, { epoch: 1, seq: 9 })).toBe(
      false,
    );
  });
});
