import { describe, expect, it } from "vitest";
import { isRoomEvent } from "./useRoomEvents";

/**
 * 契约守卫必须覆盖 shared 中全部 RoomEvent 类型。
 * 漏掉一种事件时，页面逻辑收不到该事件，实时状态会静默缺失。
 */
const EVENT_TYPES = [
  "run_started",
  "message_started",
  "message_delta",
  "message_final",
  "task_changed",
  "discussion_changed",
  "collaboration_changed",
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
});
