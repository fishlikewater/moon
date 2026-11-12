import { describe, expect, it } from "vitest";
import type { CollaborationAssignment } from "@moon/shared";
import {
  assignmentStatusLabel,
  assignmentsForRound,
  collaborationRoleLabel,
  collaborationStatusLabel,
  collaborationStopReasonLabel,
  countAssignments,
  formatAssignmentCounts,
  formatCollaborationRound,
  shouldShowRoundDivider,
} from "./collaborationView";

function assignment(
  overrides: Partial<CollaborationAssignment> = {},
): CollaborationAssignment {
  return {
    id: 1,
    batchId: 1,
    round: 1,
    agentId: 2,
    instruction: "整理方案",
    status: "queued",
    runId: null,
    messageId: null,
    error: null,
    createdAt: 1,
    finishedAt: null,
    ...overrides,
  };
}

describe("collaborationView 文案", () => {
  it("覆盖全部批次状态与停止原因", () => {
    expect(collaborationStatusLabel("running")).toBe("委派进行中");
    expect(collaborationStatusLabel("completed")).toBe("委派完成");
    expect(collaborationStatusLabel("stopped")).toBe("委派已停止");
    expect(collaborationStatusLabel("failed")).toBe("委派失败");

    expect(collaborationStopReasonLabel(null)).toBeNull();
    expect(collaborationStopReasonLabel("moderator_stop")).toBe("主持人结束");
    expect(collaborationStopReasonLabel("max_rounds")).toBe("达到轮次上限");
    expect(collaborationStopReasonLabel("user_stop")).toBe("用户手动停止");
    expect(collaborationStopReasonLabel("interrupted")).toBe("服务重启中断");
    expect(collaborationStopReasonLabel("invalid_next_assignments")).toBe(
      "下一轮指派非法",
    );
    expect(collaborationStopReasonLabel("moderator_failed")).toBe(
      "主持人复核失败",
    );
    expect(collaborationStopReasonLabel("internal_error")).toBe("内部错误");
  });

  it("展示指派状态与消息角色", () => {
    expect(assignmentStatusLabel("queued")).toBe("排队中");
    expect(assignmentStatusLabel("running")).toBe("进行中");
    expect(assignmentStatusLabel("succeeded")).toBe("成功");
    expect(assignmentStatusLabel("failed")).toBe("失败");
    expect(assignmentStatusLabel("skipped")).toBe("已跳过");

    expect(collaborationRoleLabel("assignment")).toBe("成员产出");
    expect(collaborationRoleLabel("summary")).toBe("主持人汇总");
    expect(collaborationRoleLabel("system")).toBe("委派提示");
  });

  it("轮次标签", () => {
    expect(formatCollaborationRound(1, 3)).toBe("第 1/3 轮");
    expect(formatCollaborationRound(3, 3)).toBe("第 3/3 轮");
  });
});

describe("assignmentsForRound 与统计", () => {
  const assignments = [
    assignment({ id: 1, round: 1, status: "succeeded" }),
    assignment({ id: 2, round: 1, status: "failed" }),
    assignment({ id: 3, round: 2, status: "running" }),
    assignment({ id: 4, round: 2, status: "queued" }),
    assignment({ id: 5, round: 2, status: "skipped" }),
  ];

  it("只返回当前轮次", () => {
    expect(assignmentsForRound(assignments, 1).map((item) => item.id)).toEqual([
      1, 2,
    ]);
    expect(assignmentsForRound(assignments, 2).map((item) => item.id)).toEqual([
      3, 4, 5,
    ]);
    expect(assignmentsForRound(assignments, 3)).toEqual([]);
  });

  it("按状态计数并只展示非零状态", () => {
    expect(countAssignments(assignmentsForRound(assignments, 2))).toEqual({
      total: 3,
      queued: 1,
      running: 1,
      succeeded: 0,
      failed: 0,
      skipped: 1,
    });
    expect(
      formatAssignmentCounts(countAssignments(assignmentsForRound(assignments, 2))),
    ).toBe("1 进行中 · 1 排队中 · 1 已跳过");
  });

  it("没有指派时给出占位文案", () => {
    expect(formatAssignmentCounts(countAssignments([]))).toBe("暂无指派");
  });
});

describe("shouldShowRoundDivider", () => {
  it("进入新轮次时插入分隔", () => {
    expect(shouldShowRoundDivider({ collaboration: null }, null)).toBe(false);
    expect(
      shouldShowRoundDivider({ collaboration: { batchId: 1, round: 1, role: "assignment" } }, null),
    ).toBe(true);
    expect(
      shouldShowRoundDivider(
        { collaboration: { batchId: 1, round: 1, role: "assignment" } },
        { collaboration: { batchId: 1, round: 1, role: "assignment" } },
      ),
    ).toBe(false);
    expect(
      shouldShowRoundDivider(
        { collaboration: { batchId: 1, round: 2, role: "summary" } },
        { collaboration: { batchId: 1, round: 1, role: "summary" } },
      ),
    ).toBe(true);
  });

  it("非委派消息不插入分隔", () => {
    expect(
      shouldShowRoundDivider(
        { collaboration: null },
        { collaboration: { batchId: 1, round: 1, role: "assignment" } },
      ),
    ).toBe(false);
  });
});
