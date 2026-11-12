import type { QueueJob, QueueJobKind } from "@moon/shared";

const KIND_LABELS: Record<QueueJobKind, string> = {
  chat: "聊天",
  moderator: "主持人",
  discussion: "讨论",
  collaboration: "委派",
  task: "任务",
};

export function queueKindLabel(kind: QueueJobKind): string {
  return KIND_LABELS[kind];
}

export function queueStateLabel(state: QueueJob["state"]): string {
  return state === "running" ? "运行中" : "排队中";
}

/** 队列标题后缀；读取中由调用方处理。 */
export function queueHeadline(jobs: QueueJob[], paused: boolean): string {
  if (paused && jobs.length === 0) {
    return "已暂停";
  }
  if (paused) {
    return `${jobs.length} 个 · 已暂停`;
  }
  return jobs.length === 0 ? "空闲" : `${jobs.length} 个`;
}

/** 运行中优先，其余按 position 升序；不改动入参数组。 */
export function sortQueueJobs(jobs: QueueJob[]): QueueJob[] {
  return [...jobs].sort((left, right) => {
    if (left.state !== right.state) {
      return left.state === "running" ? -1 : 1;
    }
    return left.position - right.position;
  });
}

/** 等待/运行时长：不足 1 分钟显示 Ns，之后 Nm 或 NmSs；负数按 0。 */
export function formatWait(milliseconds: number): string {
  const seconds = Math.max(0, Math.floor(milliseconds / 1000));
  if (seconds < 60) {
    return `${seconds}s`;
  }
  const minutes = Math.floor(seconds / 60);
  const rest = seconds % 60;
  return rest === 0 ? `${minutes}m` : `${minutes}m${rest}s`;
}

/** 只有排队中的普通聊天与主持人任务提供队列取消。 */
export function canCancelQueueJob(job: QueueJob): boolean {
  return (
    job.state === "queued" &&
    (job.kind === "chat" || job.kind === "moderator")
  );
}
