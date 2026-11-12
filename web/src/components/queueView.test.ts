import { describe, expect, it } from "vitest";
import type { QueueJob } from "@moon/shared";
import {
  canCancelQueueJob,
  formatWait,
  queueHeadline,
  queueKindLabel,
  queueStateLabel,
  sortQueueJobs,
} from "./queueView";

function job(
  overrides: Partial<QueueJob> & Pick<QueueJob, "id" | "kind" | "state">,
): QueueJob {
  const base: QueueJob = {
    id: overrides.id,
    roomId: 1,
    agentId: 2,
    label: `任务${overrides.id}`,
    kind: overrides.kind,
    state: overrides.state,
    position: 1,
    enqueuedAt: 1_000,
    startedAt: overrides.state === "running" ? 2_000 : null,
  };
  return { ...base, ...overrides };
}

describe("queueView", () => {
  it("映射全部任务类型文案", () => {
    expect(queueKindLabel("chat")).toBe("聊天");
    expect(queueKindLabel("moderator")).toBe("主持人");
    expect(queueKindLabel("discussion")).toBe("讨论");
    expect(queueKindLabel("collaboration")).toBe("委派");
    expect(queueKindLabel("task")).toBe("任务");
  });

  it("映射运行中与排队文案", () => {
    expect(queueStateLabel("running")).toBe("运行中");
    expect(queueStateLabel("queued")).toBe("排队中");
  });

  it("队列标题区分空闲、任务数与暂停状态", () => {
    const pausedJob = job({ id: 1, kind: "chat", state: "queued" });

    expect(queueHeadline([], false)).toBe("空闲");
    expect(queueHeadline([pausedJob], false)).toBe("1 个");
    expect(queueHeadline([], true)).toBe("已暂停");
    expect(queueHeadline([pausedJob], true)).toBe("1 个 · 已暂停");
  });

  it("运行中置顶，排队按 position 排序且不改入参", () => {
    const input = [
      job({ id: 3, kind: "chat", state: "queued", position: 2 }),
      job({ id: 1, kind: "chat", state: "running", position: 0 }),
      job({ id: 2, kind: "chat", state: "queued", position: 1 }),
    ];

    const sorted = sortQueueJobs(input);

    expect(sorted.map((item) => item.id)).toEqual([1, 2, 3]);
    expect(input.map((item) => item.id)).toEqual([3, 1, 2]);
    expect(sortQueueJobs([])).toEqual([]);
  });

  it("等待时长按秒与分钟格式化", () => {
    expect(formatWait(0)).toBe("0s");
    expect(formatWait(999)).toBe("0s");
    expect(formatWait(59_999)).toBe("59s");
    expect(formatWait(60_000)).toBe("1m");
    expect(formatWait(65_000)).toBe("1m5s");
    expect(formatWait(-5_000)).toBe("0s");
  });

  it("只有排队中的聊天与主持人可取消", () => {
    const queuedChat = job({ id: 1, kind: "chat", state: "queued" });
    const queuedModerator = job({ id: 2, kind: "moderator", state: "queued" });
    const queuedDiscussion = job({ id: 3, kind: "discussion", state: "queued" });
    const queuedTask = job({ id: 4, kind: "task", state: "queued" });
    const runningChat = job({ id: 5, kind: "chat", state: "running" });

    expect(canCancelQueueJob(queuedChat)).toBe(true);
    expect(canCancelQueueJob(queuedModerator)).toBe(true);
    expect(canCancelQueueJob(queuedDiscussion)).toBe(false);
    expect(canCancelQueueJob(queuedTask)).toBe(false);
    expect(canCancelQueueJob(runningChat)).toBe(false);
  });
});
