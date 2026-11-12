import { useEffect, type ReactNode } from "react";
import type { Task } from "@moon/shared";
import TaskTimeline from "./TaskTimeline";
import { TASK_PRIORITY_LABELS, TASK_STATUS_LABELS, formatDue } from "./taskView";

function formatTime(ms: number): string {
  return new Date(ms).toLocaleString("zh-CN", { hour12: false });
}

export interface TaskDetailPanelProps {
  task: Task;
  onClose: () => void;
  /** 任务所属房间名称；跨房间列表由页面解析后传入。 */
  roomName?: string;
  /** 指派成员名称；未指派或未解析时为空。 */
  assigneeName?: string | null;
  /** 可选的任务操作区（启动编排、状态流转），由页面提供并管理副作用。 */
  actions?: ReactNode;
}

/**
 * 任务详情侧滑抽屉（展示组件）：基础信息 + 可选操作区 + 服务端聚合的完整时间线。
 *
 * 时间线按任务 id 拉取，因此切换任务或刷新页面后仍能完整回放，不再依赖本页
 * 会话内观察到的事件。抽屉只是展示层，打开期间锁住页面滚动，关闭或卸载时恢复
 * 原值，不改变路由或数据。
 */
export default function TaskDetailPanel({
  task,
  onClose,
  roomName,
  assigneeName,
  actions,
}: TaskDetailPanelProps) {
  useEffect(() => {
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        onClose();
      }
    };
    document.addEventListener("keydown", handleKeyDown);

    return () => {
      document.body.style.overflow = previousOverflow;
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [onClose]);

  return (
    <>
      <div className="drawer-backdrop" onClick={onClose} aria-hidden="true" />
      <aside
        className="drawer"
        role="dialog"
        aria-modal="true"
        aria-label="任务详情"
      >
        <header className="drawer-head">
          <div className="drawer-title-group">
            <h2 className="drawer-title">任务详情</h2>
            <span className={`status-pill ${task.status}`}>
              {TASK_STATUS_LABELS[task.status]}
            </span>
          </div>
          <button
            type="button"
            className="btn btn-ghost btn-sm"
            aria-label="关闭任务详情"
            onClick={onClose}
          >
            关闭
          </button>
        </header>

        <div className="drawer-body">
          <dl className="detail-fields">
            <div className="detail-row">
              <dt>任务</dt>
              <dd>{task.title}</dd>
            </div>
            <div className="detail-row">
              <dt>编号</dt>
              <dd>#{task.id}</dd>
            </div>
            {roomName !== undefined && (
              <div className="detail-row">
                <dt>房间</dt>
                <dd>{roomName}</dd>
              </div>
            )}
            <div className="detail-row">
              <dt>状态</dt>
              <dd>{TASK_STATUS_LABELS[task.status]}</dd>
            </div>
            <div className="detail-row">
              <dt>优先级</dt>
              <dd>{TASK_PRIORITY_LABELS[task.priority]}</dd>
            </div>
            {task.assigneeAgentId !== null && (
              <div className="detail-row">
                <dt>指派人</dt>
                <dd>
                  {assigneeName ?? `智能体 #${task.assigneeAgentId}`}
                </dd>
              </div>
            )}
            {task.dueAt !== null && (
              <div className="detail-row">
                <dt>截止</dt>
                <dd
                  className={
                    task.dueAt < Date.now() &&
                    task.status !== "done" &&
                    task.status !== "cancelled"
                      ? "detail-overdue"
                      : undefined
                  }
                >
                  {formatTime(task.dueAt)}
                  {formatDue(task.dueAt, Date.now()) !== null
                    ? `（${formatDue(task.dueAt, Date.now())}）`
                    : ""}
                </dd>
              </div>
            )}
            {task.scheduledAt !== null && (
              <div className="detail-row">
                <dt>定时启动</dt>
                <dd>{formatTime(task.scheduledAt)}</dd>
              </div>
            )}
            {task.createdByAgentId !== null && (
              <div className="detail-row">
                <dt>来源</dt>
                <dd>内置主持人创建（智能体 #{task.createdByAgentId}）</dd>
              </div>
            )}
            <div className="detail-row">
              <dt>创建时间</dt>
              <dd>{formatTime(task.createdAt)}</dd>
            </div>
            <div className="detail-row">
              <dt>更新时间</dt>
              <dd>{formatTime(task.updatedAt)}</dd>
            </div>
            {task.description !== "" && (
              <div className="detail-row">
                <dt>描述</dt>
                <dd className="detail-desc">{task.description}</dd>
              </div>
            )}
          </dl>

          {actions !== undefined && (
            <section className="detail-actions" aria-label="任务操作">
              <h3>任务操作</h3>
              <div className="detail-actions-row">{actions}</div>
            </section>
          )}

          {task.plan !== null && (
            <section className="detail-plan" aria-label="任务计划">
              <h3>任务计划</h3>
              <div className="plan-block">
                <h4>目标</h4>
                <p>{task.plan.goal}</p>
              </div>
              <div className="plan-block">
                <h4>验收标准</h4>
                <ul>
                  {task.plan.acceptanceCriteria.map((item, index) => (
                    <li key={`ac-${index}`}>{item}</li>
                  ))}
                </ul>
              </div>
              <div className="plan-block">
                <h4>验证方式</h4>
                <ul>
                  {task.plan.verification.map((item, index) => (
                    <li key={`verify-${index}`}>{item}</li>
                  ))}
                </ul>
              </div>
              <div className="plan-block">
                <h4>范围边界</h4>
                {task.plan.scope.length === 0 ? (
                  <p className="muted small">未显式限定范围。</p>
                ) : (
                  <ul>
                    {task.plan.scope.map((item, index) => (
                      <li key={`scope-${index}`}>{item}</li>
                    ))}
                  </ul>
                )}
              </div>
            </section>
          )}

          <TaskTimeline taskId={task.id} refreshToken={task.updatedAt} />
        </div>
      </aside>
    </>
  );
}
