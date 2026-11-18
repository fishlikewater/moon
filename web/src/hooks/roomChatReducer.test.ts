import { describe, expect, it } from "vitest";
import type { Message, RoomEvent, RoomState } from "@moon/shared";
import {
  initialRoomChatState,
  mergeMessages,
  roomChatReducer,
  type RoomChatAction,
  type RoomChatState,
} from "./roomChatReducer";

const ROOM = 3;

function message(overrides: Partial<Message> = {}): Message {
  return {
    id: 1,
    roomId: ROOM,
    seq: 1,
    senderType: "agent",
    senderAgentId: 7,
    status: "streaming",
    kind: "reply",
    runId: 41,
    content: "",
    taskId: null,
    replyToMessageId: null,
    discussion: null,
    collaboration: null,
    attachments: [],
    reactions: [],
    createdAt: 1,
    ...overrides,
  };
}

function eventAction(
  payload: RoomEvent,
  atBottom = true,
  now = 1_000,
): RoomChatAction {
  return {
    kind: "event",
    event: payload,
    roomId: ROOM,
    atBottom,
    now,
  };
}

function apply(state: RoomChatState, action: RoomChatAction): RoomChatState {
  return roomChatReducer(state, action).state;
}

describe("roomChatReducer / 消息流", () => {
  it("message_delta 只累积到目标消息", () => {
    let state = apply(initialRoomChatState, {
      kind: "history",
      messages: [
        message({ id: 1, seq: 1, content: "你好" }),
        message({ id: 2, seq: 2, content: "另一条" }),
      ],
    });

    state = apply(
      state,
      eventAction({
        type: "message_delta",
        roomId: ROOM,
        messageId: 1,
        text: "，世界",
      }),
    );

    expect(state.messages.find((item) => item.id === 1)?.content).toBe(
      "你好，世界",
    );
    expect(state.messages.find((item) => item.id === 2)?.content).toBe("另一条");
  });

  it("迟到的 message_final 即使正文更短也覆盖 streaming 版本", () => {
    const streamed = message({
      id: 5,
      seq: 5,
      status: "streaming",
      content: "很长很长的一段流式增量文本",
    });
    const finalized = message({
      id: 5,
      seq: 5,
      status: "final",
      content: "短",
    });

    let state = apply(initialRoomChatState, {
      kind: "history",
      messages: [streamed],
    });
    state = apply(
      state,
      eventAction({ type: "message_final", roomId: ROOM, message: finalized }),
    );

    expect(state.messages).toHaveLength(1);
    expect(state.messages[0].status).toBe("final");
    expect(state.messages[0].content).toBe("短");
  });

  it("message_started 追加阶段消息并登记运行归属", () => {
    const phase = message({ id: 9, seq: 9, runId: null });

    const state = apply(
      initialRoomChatState,
      eventAction({
        type: "message_started",
        roomId: ROOM,
        runId: 33,
        message: phase,
      }),
    );

    expect(state.runIdByMessageId[9]).toBe(33);
    expect(state.messages.map((item) => item.id)).toEqual([9]);
    // 阶段切换不改变运行登记本身。
    expect(state.activeRuns).toEqual({});
  });

  it("message_reactions_changed 只替换目标消息的反应", () => {
    let state = apply(initialRoomChatState, {
      kind: "history",
      messages: [
        message({ id: 1, seq: 1 }),
        message({ id: 2, seq: 2 }),
      ],
    });
    const reactions = [{ emoji: "👍", count: 2, reactedByMe: true }];

    state = apply(
      state,
      eventAction({
        type: "message_reactions_changed",
        roomId: ROOM,
        messageId: 2,
        reactions,
      }),
    );

    expect(state.messages[0].reactions).toEqual([]);
    expect(state.messages[1].reactions).toEqual(reactions);
  });
});

describe("roomChatReducer / 运行生命周期", () => {
  it("run_started 登记运行、播报并刷新队列", () => {
    const start = message({ id: 1, seq: 1, runId: 41 });

    const result = roomChatReducer(
      initialRoomChatState,
      eventAction(
        {
          type: "run_started",
          roomId: ROOM,
          runId: 41,
          agentId: 7,
          messageId: 1,
          message: start,
        },
        true,
        1_000,
      ),
    );

    expect(result.state.activeRuns[41]).toEqual({
      runId: 41,
      agentId: 7,
      messageId: 1,
      startedAt: 1_000,
    });
    expect(result.state.announcement).toBe("成员开始回复");
    expect(result.state.messages.map((item) => item.id)).toEqual([1]);
    expect(result.effects).toEqual([{ kind: "refresh", target: "queue" }]);
  });

  it("progress 终态不结束运行，reply 终态结束并按 now 记录耗时", () => {
    const start = message({ id: 1, seq: 1, runId: 41 });
    let state = apply(
      initialRoomChatState,
      eventAction(
        {
          type: "run_started",
          roomId: ROOM,
          runId: 41,
          agentId: 7,
          messageId: 1,
          message: start,
        },
        true,
        1_000,
      ),
    );
    expect(Object.keys(state.activeRuns)).toEqual(["41"]);

    state = apply(
      state,
      eventAction(
        {
          type: "message_final",
          roomId: ROOM,
          message: message({
            id: 2,
            seq: 2,
            runId: 41,
            kind: "progress",
            status: "final",
            content: "阶段总结",
          }),
        },
        true,
        2_000,
      ),
    );
    expect(Object.keys(state.activeRuns)).toEqual(["41"]);
    expect(state.runDurations).toEqual({});

    state = apply(
      state,
      eventAction(
        {
          type: "message_final",
          roomId: ROOM,
          message: message({
            id: 3,
            seq: 3,
            runId: 41,
            kind: "reply",
            status: "final",
            content: "最终回复",
          }),
        },
        true,
        3_000,
      ),
    );
    expect(state.activeRuns).toEqual({});
    expect(state.runDurations[41]).toBe(2_000);
  });

  it("不贴底时累积未读数，贴底时不累积；mark_seen 清零", () => {
    const reply = message({
      id: 2,
      seq: 2,
      status: "final",
      content: "回复",
    });

    const away = apply(
      initialRoomChatState,
      eventAction(
        { type: "message_final", roomId: ROOM, message: reply },
        false,
      ),
    );
    expect(away.unseenCount).toBe(1);
    expect(away.announcement).toBe("成员回复已完成");

    const atBottom = apply(
      initialRoomChatState,
      eventAction({ type: "message_final", roomId: ROOM, message: reply }, true),
    );
    expect(atBottom.unseenCount).toBe(0);

    expect(apply(away, { kind: "mark_seen" }).unseenCount).toBe(0);
  });
});

describe("roomChatReducer / 房间与同步", () => {
  it("跨房间事件被丢弃，不改变状态也不产生副作用", () => {
    const result = roomChatReducer(
      initialRoomChatState,
      eventAction({
        type: "message_delta",
        roomId: ROOM + 1,
        messageId: 1,
        text: "别的房间",
      }),
    );

    expect(result.state).toBe(initialRoomChatState);
    expect(result.effects).toEqual([]);
  });

  it("state_changed 上抛完整快照，聊天状态不变", () => {
    const taskState: RoomState = {
      unit: "task",
      snapshot: {
        id: 8,
        roomId: ROOM,
        title: "看板任务",
        description: "",
        status: "in_progress",
        priority: "normal",
        assigneeAgentId: null,
        dueAt: null,
        scheduledAt: null,
        plan: null,
        createdByAgentId: 1,
        createdAt: 1,
        updatedAt: 2,
      },
    };

    const result = roomChatReducer(
      initialRoomChatState,
      eventAction({ type: "state_changed", roomId: ROOM, state: taskState }),
    );

    expect(result.state).toBe(initialRoomChatState);
    expect(result.effects).toEqual([{ kind: "state", state: taskState }]);
  });

  it("history_cleared 触发三项刷新与整体重拉", () => {
    const result = roomChatReducer(
      initialRoomChatState,
      eventAction({ type: "history_cleared", roomId: ROOM }),
    );

    expect(result.effects).toContainEqual({
      kind: "refresh",
      target: "context",
    });
    expect(result.effects).toContainEqual({ kind: "refresh", target: "queue" });
    expect(result.effects).toContainEqual({ kind: "refresh", target: "files" });
    expect(result.effects).toContainEqual({ kind: "reload_messages" });
    expect(result.effects).toHaveLength(4);
  });

  it("resync 合并历史、按 streaming 重建运行并派发页面重拉", () => {
    const local = message({
      id: 1,
      seq: 1,
      runId: 41,
      status: "streaming",
      content: "本地增量更长",
    });
    let state = apply(initialRoomChatState, {
      kind: "history",
      messages: [local],
    });

    const result = roomChatReducer(state, {
      kind: "resync",
      messages: [
        message({
          id: 1,
          seq: 1,
          runId: 41,
          status: "streaming",
          content: "服务端",
        }),
        message({
          id: 2,
          seq: 2,
          runId: null,
          status: "final",
          senderAgentId: null,
          content: "已完成",
        }),
      ],
      now: 9_000,
    });

    // 合并保持单调：本地更长的流式正文不会被服务端旧快照截短。
    expect(result.state.messages.find((item) => item.id === 1)?.content).toBe(
      "本地增量更长",
    );
    expect(result.state.activeRuns[41]).toEqual({
      runId: 41,
      agentId: 7,
      messageId: 1,
      startedAt: 9_000,
    });
    expect(result.effects).toContainEqual({ kind: "resync" });
    expect(result.effects).toContainEqual({
      kind: "refresh",
      target: "context",
    });
  });

  it("resync 拉取失败时仍派发页面重拉", () => {
    const result = roomChatReducer(initialRoomChatState, {
      kind: "resync",
      messages: null,
      now: 1_000,
    });

    expect(result.state).toBe(initialRoomChatState);
    expect(result.effects).toContainEqual({ kind: "resync" });
  });

  it("reset 清空消息与运行登记", () => {
    const busy = apply(
      initialRoomChatState,
      eventAction({
        type: "run_started",
        roomId: ROOM,
        runId: 41,
        agentId: 7,
        messageId: 1,
        message: message({ id: 1, seq: 1, runId: 41 }),
      }),
    );
    expect(Object.keys(busy.activeRuns)).toEqual(["41"]);

    expect(apply(busy, { kind: "reset" })).toBe(initialRoomChatState);
  });
});

describe("mergeMessages", () => {
  it("按 id 去重并按 seq 升序", () => {
    const merged = mergeMessages(
      [message({ id: 2, seq: 2, content: "b" })],
      [message({ id: 1, seq: 1, content: "a" })],
    );

    expect(merged.map((item) => item.id)).toEqual([1, 2]);
  });
});
