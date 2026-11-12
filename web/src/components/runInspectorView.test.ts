import { describe, expect, it } from "vitest";
import type { Agent, AgentRun, Message, RunActivity } from "@moon/shared";
import {
  activityDetailPreview,
  buildRunList,
  buildRunTimeline,
  describeAgentModel,
  formatActivityOffset,
  formatRunDuration,
  mergeActivities,
  runStatusLabel,
} from "./runInspectorView";

function activity(seq: number, detail: string | null = null): RunActivity {
  return {
    runId: 1,
    seq,
    kind: "tool",
    title: `活动 ${seq}`,
    detail,
    at: 1_000 + seq * 500,
  };
}

function run(overrides: Partial<AgentRun>): AgentRun {
  return {
    id: 1,
    roomId: 1,
    agentId: 1,
    taskId: null,
    status: "succeeded",
    prompt: "指令",
    result: "结果",
    error: null,
    harnessSessionId: null,
    createdAt: 1_000,
    finishedAt: 61_000,
    ...overrides,
  };
}

describe("mergeActivities", () => {
  it("dedupes by seq, prefers live values and sorts ascending", () => {
    const fetched = [activity(1), activity(3)];
    const live = [activity(3, "实时"), activity(2)];

    const merged = mergeActivities(fetched, live);

    expect(merged.map((item) => item.seq)).toEqual([1, 2, 3]);
    expect(merged[2]!.detail).toBe("实时");
  });

  it("returns fetched snapshots unchanged when no live activity exists", () => {
    const fetched = [activity(2), activity(1)];

    expect(mergeActivities(fetched, []).map((item) => item.seq)).toEqual([
      1, 2,
    ]);
  });
});

describe("describeAgentModel", () => {
  it("falls back to the harness default and appends reasoning level", () => {
    const base: Agent = {
      id: 1,
      name: "a",
      persona: "",
      harness: "zcode",
      customHarnessId: null,
      workspaceDir: null,
      modelConfig: null,
      builtinRole: null,
      createdAt: 0,
    };

    expect(describeAgentModel(base)).toBe("默认模型");
    expect(
      describeAgentModel({
        ...base,
        modelConfig: { model: "deepseek-v4.1-flash", reasoningLevel: "max" },
      }),
    ).toBe("deepseek-v4.1-flash · max");
    expect(
      describeAgentModel({ ...base, modelConfig: { model: "m" } }),
    ).toBe("m");
    expect(describeAgentModel(undefined)).toBe("未知智能体");
  });
});

describe("formatRunDuration", () => {
  it("uses finishedAt for completed runs and now for running runs", () => {
    expect(formatRunDuration(run({}), 999_999)).toBe("01:00");
    expect(
      formatRunDuration(
        run({ status: "running", finishedAt: null }),
        31_000,
      ),
    ).toBe("00:30");
  });
});

describe("formatActivityOffset", () => {
  it("formats seconds and minutes relative to the run start", () => {
    expect(formatActivityOffset(3_450, 1_000)).toBe("+2.5s");
    expect(formatActivityOffset(75_000, 1_000)).toBe("+1m14s");
    expect(formatActivityOffset(0, 1_000)).toBe("+0.0s");
  });
});

describe("buildRunList", () => {
  it("puts running runs first and dedupes by runId", () => {
    const items = buildRunList(
      [{ runId: 2, agentId: 1, startedAt: 5_000 }],
      [run({ id: 1, createdAt: 6_000 }), run({ id: 2, createdAt: 4_000 })],
    );

    expect(items.map((item) => item.runId)).toEqual([2, 1]);
    expect(items[0]!.run?.id).toBe(2);
  });
});

describe("activityDetailPreview", () => {
  it("collapses whitespace and marks truncation", () => {
    expect(activityDetailPreview("a\n\n b")).toEqual({
      text: "a b",
      truncated: false,
    });
    expect(activityDetailPreview(null)).toEqual({ text: "", truncated: false });

    const long = activityDetailPreview("x".repeat(200), 10);
    expect(long.truncated).toBe(true);
    expect(long.text).toBe(`${"x".repeat(10)}…`);
  });
});

describe("runStatusLabel", () => {
  it("labels every run status", () => {
    expect(runStatusLabel("running")).toBe("运行中");
    expect(runStatusLabel("failed")).toBe("失败");
  });
});

function message(
  id: number,
  kind: Message["kind"],
  content: string,
  createdAt: number,
): Message {
  return {
    id,
    roomId: 1,
    seq: id,
    senderType: "agent",
    senderAgentId: 1,
    status: "final",
    kind,
    runId: 1,
    content,
    taskId: null,
    createdAt,
  };
}

describe("buildRunTimeline", () => {
  it("interleaves messages and activities by time and dedupes both", () => {
    const messages = [
      message(1, "progress", "阶段一", 1_500),
      message(2, "reply", "结论", 2_500),
      message(2, "reply", "结论", 2_500),
    ];
    const activities = [
      activity(2),
      activity(1),
      activity(1),
    ];

    const entries = buildRunTimeline(messages, activities);

    expect(entries.map((entry) => entry.key)).toEqual([
      "message-1",
      "activity-1",
      "activity-2",
      "message-2",
    ]);
  });

  it("puts a message before an activity at the same timestamp", () => {
    const entries = buildRunTimeline(
      [message(1, "progress", "阶段", 2_000)],
      [{ ...activity(1), at: 2_000 }],
    );

    expect(entries.map((entry) => entry.kind)).toEqual([
      "message",
      "activity",
    ]);
  });
});
