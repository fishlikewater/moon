/**
 * 委派批次相关的展示文案与判定。
 *
 * 服务端只存批次/指派状态，界面文案集中在这里，避免 RoomPage 内联
 * 大量条件分支；保持纯函数，便于单独测试。
 */
import type {
  CollaborationAssignment,
  CollaborationAssignmentStatus,
  CollaborationMessageRole,
  CollaborationStatus,
  CollaborationStopReason,
  Message,
} from "@moon/shared";

const STATUS_LABELS: Record<CollaborationStatus, string> = {
  running: "委派进行中",
  completed: "委派完成",
  stopped: "委派已停止",
  failed: "委派失败",
};

const STOP_REASON_LABELS: Record<CollaborationStopReason, string> = {
  moderator_stop: "主持人结束",
  max_rounds: "达到轮次上限",
  user_stop: "用户手动停止",
  interrupted: "服务重启中断",
  invalid_next_assignments: "下一轮指派非法",
  moderator_failed: "主持人复核失败",
  internal_error: "内部错误",
};

const ASSIGNMENT_STATUS_LABELS: Record<CollaborationAssignmentStatus, string> =
  {
    queued: "排队中",
    running: "进行中",
    succeeded: "成功",
    failed: "失败",
    skipped: "已跳过",
  };

const ROLE_LABELS: Record<CollaborationMessageRole, string> = {
  assignment: "成员产出",
  summary: "主持人汇总",
  system: "委派提示",
};

export interface CollaborationAssignmentCounts {
  total: number;
  queued: number;
  running: number;
  succeeded: number;
  failed: number;
  skipped: number;
}

export function collaborationStatusLabel(status: CollaborationStatus): string {
  return STATUS_LABELS[status];
}

export function collaborationStopReasonLabel(
  reason: CollaborationStopReason | null,
): string | null {
  return reason === null ? null : STOP_REASON_LABELS[reason];
}

export function assignmentStatusLabel(
  status: CollaborationAssignmentStatus,
): string {
  return ASSIGNMENT_STATUS_LABELS[status];
}

export function collaborationRoleLabel(
  role: CollaborationMessageRole,
): string {
  return ROLE_LABELS[role];
}

export function formatCollaborationRound(
  currentRound: number,
  maxRounds: number,
): string {
  return `第 ${currentRound}/${maxRounds} 轮`;
}

/** 只统计指定轮次的指派；状态条按当前轮展示，历史轮次留在消息流里。 */
export function assignmentsForRound(
  assignments: readonly CollaborationAssignment[],
  round: number,
): CollaborationAssignment[] {
  return assignments.filter((assignment) => assignment.round === round);
}

export function countAssignments(
  assignments: readonly CollaborationAssignment[],
): CollaborationAssignmentCounts {
  const counts: CollaborationAssignmentCounts = {
    total: assignments.length,
    queued: 0,
    running: 0,
    succeeded: 0,
    failed: 0,
    skipped: 0,
  };
  for (const assignment of assignments) {
    counts[assignment.status] += 1;
  }
  return counts;
}

/** 只展示非零状态，避免状态条出现一串“0 失败”。 */
export function formatAssignmentCounts(
  counts: CollaborationAssignmentCounts,
): string {
  if (counts.total === 0) {
    return "暂无指派";
  }
  const parts: string[] = [];
  if (counts.running > 0) {
    parts.push(`${counts.running} 进行中`);
  }
  if (counts.queued > 0) {
    parts.push(`${counts.queued} 排队中`);
  }
  if (counts.succeeded > 0) {
    parts.push(`${counts.succeeded} 成功`);
  }
  if (counts.failed > 0) {
    parts.push(`${counts.failed} 失败`);
  }
  if (counts.skipped > 0) {
    parts.push(`${counts.skipped} 已跳过`);
  }
  return parts.join(" · ");
}

/**
 * 消息流里的轮次分隔只在轮次变化时插入一次。
 *
 * 传入的是当前筛选后的上一条可见消息：筛选把该轮首条隐藏后，仍在
 * 该轮第一条可见消息前显示分隔，避免用户失去轮次上下文。
 */
export function shouldShowRoundDivider(
  message: Pick<Message, "collaboration">,
  previous: Pick<Message, "collaboration"> | null,
): boolean {
  const round = message.collaboration?.round ?? null;
  if (round === null) {
    return false;
  }
  return (previous?.collaboration?.round ?? null) !== round;
}
