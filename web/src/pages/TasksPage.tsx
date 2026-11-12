import {
  useCallback,
  useEffect,
  useMemo,
  useState,
  type FormEvent,
  type ReactNode,
} from "react";
import { Link, useSearchParams } from "react-router-dom";
import type { Agent, Room, Task, TaskPriority, TaskStatus } from "@moon/shared";
import { api, errorMessage } from "../api/client";
import { EmptyState } from "../components/layout/EmptyState";
import { shouldOpenNew, withoutNewParam } from "./newEntry";
import { ListSkeleton } from "../components/layout/ListSkeleton";
import { PageHeader } from "../components/layout/PageHeader";
import TaskDetailPanel from "../components/TaskDetailPanel";
import {
  TASK_PRIORITY_LABELS,
  TASK_STATUS_LABELS,
  TASK_TRANSITIONS,
  TASK_TRANSITION_LABELS,
  canActivateTask,
  filterTasksByRoom,
  formatDue,
  formatSchedule,
  fromDatetimeLocal,
  groupTasksByStatus,
  mergeTasks,
  parseTaskView,
  toDatetimeLocal,
  type TaskView,
} from "../components/taskView";

/** 房间筛选的「全部房间」哨兵值；数字字符串才是具体房间。 */
const ALL_ROOMS = "all";
/** 列表/看板视图记忆键。 */
const TASK_VIEW_STORAGE_KEY = "moon.tasks.view";
/** 表单优先级选项顺序（低 → 紧急）。 */
const PRIORITY_OPTIONS: readonly TaskPriority[] = [
  "low",
  "normal",
  "high",
  "urgent",
];

function readTaskView(): TaskView {
  try {
    return parseTaskView(window.localStorage.getItem(TASK_VIEW_STORAGE_KEY));
  } catch {
    return "list";
  }
}

function persistTaskView(view: TaskView): void {
  try {
    window.localStorage.setItem(TASK_VIEW_STORAGE_KEY, view);
  } catch {
    // 隐私模式等写入失败时只影响下次默认视图。
  }
}

function formatTime(ms: number): string {
  return new Date(ms).toLocaleString("zh-CN", { hour12: false });
}

/**
 * 任务页：跨房间任务查询、创建与操作入口。
 *
 * 服务端没有跨房间任务聚合接口，页面先取房间列表，再按房间拉取任务并在前端
 * 合并、按更新时间排序；房间筛选在本地完成，不额外请求。
 */
export default function TasksPage() {
  const [rooms, setRooms] = useState<Room[]>([]);
  const [tasks, setTasks] = useState<Task[]>([]);
  const [roomFilter, setRoomFilter] = useState<number | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const [selectedTaskId, setSelectedTaskId] = useState<number | null>(null);
  const [busyTaskId, setBusyTaskId] = useState<number | null>(null);
  const [activatingTaskId, setActivatingTaskId] = useState<number | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);

  const [paused, setPaused] = useState(false);
  const [pauseLoading, setPauseLoading] = useState(false);
  const [pausing, setPausing] = useState(false);

  const [agents, setAgents] = useState<Agent[]>([]);
  const [view, setView] = useState<TaskView>(() => readTaskView());

  const [createOpen, setCreateOpen] = useState(false);
  const [editingTaskId, setEditingTaskId] = useState<number | null>(null);
  const [formRoomId, setFormRoomId] = useState<string>("");
  const [formTitle, setFormTitle] = useState("");
  const [formDescription, setFormDescription] = useState("");
  const [formPriority, setFormPriority] = useState<TaskPriority>("normal");
  const [formAssigneeId, setFormAssigneeId] = useState<string>("");
  const [formDueAt, setFormDueAt] = useState("");
  const [formScheduledAt, setFormScheduledAt] = useState("");
  const [formError, setFormError] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);

  const load = useCallback(async () => {
    setRefreshing(true);
    setError(null);
    try {
      const [roomList, agentList] = await Promise.all([
        api.rooms.list(),
        api.agents.list(),
      ]);
      const taskLists = await Promise.all(
        roomList.map((room) => api.tasks.list(room.id)),
      );
      setRooms(roomList);
      setAgents(agentList);
      setTasks(mergeTasks(taskLists));
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setRefreshing(false);
      setLoaded(true);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  // 暂停是房间级状态：只在筛到具体房间时读取，切换房间后重新水合。
  useEffect(() => {
    if (roomFilter === null) {
      setPaused(false);
      return;
    }

    let cancelled = false;
    setPauseLoading(true);
    void api.orchestration
      .getState(roomFilter)
      .then((state) => {
        if (!cancelled) {
          setPaused(state.paused);
        }
      })
      .catch((cause) => {
        if (!cancelled) {
          setActionError(errorMessage(cause));
        }
      })
      .finally(() => {
        if (!cancelled) {
          setPauseLoading(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [roomFilter]);

  useEffect(() => {
    if (!createOpen) {
      return;
    }
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !creating) {
        setCreateOpen(false);
      }
    };
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [createOpen, creating]);

  const roomNames = useMemo(
    () => new Map(rooms.map((room) => [room.id, room.name])),
    [rooms],
  );
  const agentNames = useMemo(
    () => new Map(agents.map((agent) => [agent.id, agent.name])),
    [agents],
  );
  const switchView = (next: TaskView) => {
    setView(next);
    persistTaskView(next);
  };
  const visibleTasks = useMemo(
    () => filterTasksByRoom(tasks, roomFilter),
    [tasks, roomFilter],
  );
  const selectedTask =
    selectedTaskId === null
      ? null
      : (tasks.find((task) => task.id === selectedTaskId) ?? null);

  const applyTask = useCallback((updated: Task) => {
    setTasks((previous) =>
      mergeTasks([
        previous.map((item) => (item.id === updated.id ? updated : item)),
      ]),
    );
  }, []);

  const resetFormFields = () => {
    setFormTitle("");
    setFormDescription("");
    setFormPriority("normal");
    setFormAssigneeId("");
    setFormDueAt("");
    setFormScheduledAt("");
    setFormError(null);
  };

  const openCreate = () => {
    setEditingTaskId(null);
    setFormRoomId(String(roomFilter ?? rooms[0]?.id ?? ""));
    resetFormFields();
    setCreateOpen(true);
  };

  const openEdit = (task: Task) => {
    setEditingTaskId(task.id);
    setFormRoomId(String(task.roomId));
    setFormTitle(task.title);
    setFormDescription(task.description);
    setFormPriority(task.priority);
    setFormAssigneeId(
      task.assigneeAgentId === null ? "" : String(task.assigneeAgentId),
    );
    setFormDueAt(toDatetimeLocal(task.dueAt));
    setFormScheduledAt(toDatetimeLocal(task.scheduledAt));
    setFormError(null);
    setCreateOpen(true);
  };

  const [searchParams, setSearchParams] = useSearchParams();

  // 命令面板「新建任务」入口：数据加载完成后打开一次。
  useEffect(() => {
    if (!loaded || !shouldOpenNew(searchParams)) {
      return;
    }
    openCreate();
    setSearchParams(withoutNewParam(searchParams), { replace: true });
    // openCreate 依赖当前房间筛选；参数清理后不会再次触发。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loaded, searchParams]);

  const closeCreate = () => {
    if (!creating) {
      setCreateOpen(false);
      setEditingTaskId(null);
    }
  };

  const submitTask = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (creating) {
      return;
    }

    const targetRoomId = Number(formRoomId);
    if (!Number.isSafeInteger(targetRoomId) || targetRoomId <= 0) {
      setFormError("请选择任务所属房间");
      return;
    }
    const title = formTitle.trim();
    if (title === "") {
      setFormError("请输入任务标题");
      return;
    }

    const editingTask =
      editingTaskId === null
        ? null
        : (tasks.find((item) => item.id === editingTaskId) ?? null);
    const scheduledAt = fromDatetimeLocal(formScheduledAt);

    setCreating(true);
    setFormError(null);
    try {
      if (editingTaskId === null) {
        const created = await api.tasks.create(targetRoomId, {
          title,
          description: formDescription.trim(),
          priority: formPriority,
          assigneeAgentId:
            formAssigneeId === "" ? null : Number(formAssigneeId),
          dueAt: fromDatetimeLocal(formDueAt),
          scheduledAt,
        });
        setCreateOpen(false);
        setRoomFilter(created.roomId);
        await load();
      } else {
        const updated = await api.tasks.update(editingTaskId, {
          title,
          description: formDescription.trim(),
          priority: formPriority,
          assigneeAgentId:
            formAssigneeId === "" ? null : Number(formAssigneeId),
          dueAt: fromDatetimeLocal(formDueAt),
          // 非待办任务不能设置定时：保持原值，不发送该字段。
          ...(editingTask !== null && editingTask.status !== "backlog"
            ? {}
            : { scheduledAt }),
        });
        applyTask(updated);
        setCreateOpen(false);
        setEditingTaskId(null);
      }
    } catch (cause) {
      setFormError(errorMessage(cause));
    } finally {
      setCreating(false);
    }
  };

  const transition = async (task: Task, status: TaskStatus) => {
    if (
      status === "cancelled" &&
      !window.confirm(`确认取消任务「${task.title}」？取消后不可恢复。`)
    ) {
      return;
    }
    setActionError(null);
    setBusyTaskId(task.id);
    try {
      applyTask(await api.tasks.updateStatus(task.id, status));
    } catch (cause) {
      // 409 INVALID_TRANSITION 等业务失败以文案呈现，不做本地猜测性改状态。
      setActionError(errorMessage(cause));
    } finally {
      setBusyTaskId(null);
    }
  };

  const activate = async (task: Task) => {
    setActionError(null);
    setActivatingTaskId(task.id);
    try {
      applyTask(await api.tasks.activate(task.id));
    } catch (cause) {
      // 409（状态不允许 / 主持人非法 / 已在运行）以文案呈现。
      setActionError(errorMessage(cause));
    } finally {
      setActivatingTaskId(null);
    }
  };

  const togglePause = async () => {
    if (roomFilter === null || pausing) {
      return;
    }
    setPausing(true);
    setActionError(null);
    try {
      const state = paused
        ? await api.orchestration.resume(roomFilter)
        : await api.orchestration.pause(roomFilter);
      setPaused(state.paused);
    } catch (cause) {
      setActionError(errorMessage(cause));
    } finally {
      setPausing(false);
    }
  };

  const detailActions: ReactNode =
    selectedTask === null ? undefined : (
      <>
        {canActivateTask(selectedTask) && (
          <button
            type="button"
            className="btn btn-primary btn-sm"
            disabled={
              busyTaskId === selectedTask.id ||
              activatingTaskId === selectedTask.id
            }
            onClick={() => void activate(selectedTask)}
          >
            {activatingTaskId === selectedTask.id ? "启动中…" : "启动编排"}
          </button>
        )}
        {TASK_TRANSITIONS[selectedTask.status].map((status) => (
          <button
            key={status}
            type="button"
            className={
              status === "cancelled"
                ? "btn btn-ghost btn-sm danger-text"
                : "btn btn-secondary btn-sm"
            }
            disabled={
              busyTaskId === selectedTask.id ||
              activatingTaskId === selectedTask.id
            }
            onClick={() => void transition(selectedTask, status)}
          >
            {busyTaskId === selectedTask.id
              ? "处理中…"
              : TASK_TRANSITION_LABELS[status]}
          </button>
        ))}
        <button
          type="button"
          className="btn btn-secondary btn-sm"
          onClick={() => openEdit(selectedTask)}
        >
          编辑
        </button>
        {!canActivateTask(selectedTask) &&
          TASK_TRANSITIONS[selectedTask.status].length === 0 && (
            <p className="muted small">终态，无可用操作。</p>
          )}
      </>
    );

  const createDisabled = rooms.length === 0;
  const formRoom =
    rooms.find((room) => String(room.id) === formRoomId) ?? null;
  const roomMembers =
    formRoom === null
      ? []
      : agents.filter((agent) => formRoom.memberIds.includes(agent.id));
  const editingTask =
    editingTaskId === null
      ? null
      : (tasks.find((item) => item.id === editingTaskId) ?? null);
  const scheduleDisabled =
    editingTask !== null && editingTask.status !== "backlog";

  return (
    <section className="page tasks-page">
      <PageHeader
        title="任务"
        subtitle="跨房间查看任务状态，创建任务时选择归属房间。"
        actions={
          <>
            <button
              type="button"
              className="btn btn-secondary btn-sm"
              disabled={refreshing}
              onClick={() => void load()}
            >
              {refreshing ? "刷新中…" : "刷新"}
            </button>
            <button
              type="button"
              className="btn btn-primary btn-sm"
              disabled={createDisabled}
              title={createDisabled ? "请先创建房间" : undefined}
              onClick={openCreate}
            >
              创建任务
            </button>
          </>
        }
      />

      {error !== null && (
        <p className="banner error" role="alert">
          {error}
          <button
            type="button"
            className="btn btn-ghost btn-sm"
            onClick={() => void load()}
          >
            重试
          </button>
        </p>
      )}

      {actionError !== null && (
        <p className="banner error" role="alert">
          {actionError}
        </p>
      )}

      <div className="tasks-toolbar">
        <label className="tasks-filter">
          <span className="field-label">房间</span>
          <select
            value={roomFilter === null ? ALL_ROOMS : String(roomFilter)}
            onChange={(event) => {
              const value = event.target.value;
              setRoomFilter(value === ALL_ROOMS ? null : Number(value));
              setSelectedTaskId(null);
            }}
            aria-label="按房间筛选任务"
          >
            <option value={ALL_ROOMS}>全部房间</option>
            {rooms.map((room) => (
              <option key={room.id} value={room.id}>
                {room.name}
              </option>
            ))}
          </select>
        </label>

        <span className="muted small tasks-count">
          共 {visibleTasks.length} 个任务
        </span>

        <div className="task-view-switch" role="group" aria-label="任务视图">
          <button
            type="button"
            className="btn btn-sm"
            aria-pressed={view === "list"}
            onClick={() => switchView("list")}
          >
            列表
          </button>
          <button
            type="button"
            className="btn btn-sm"
            aria-pressed={view === "board"}
            onClick={() => switchView("board")}
          >
            看板
          </button>
        </div>

        {roomFilter !== null && (
          <button
            type="button"
            className="btn btn-secondary btn-sm"
            disabled={pauseLoading || pausing}
            onClick={() => void togglePause()}
          >
            {pausing
              ? "处理中…"
              : paused
                ? "恢复编排"
                : "暂停编排"}
          </button>
        )}
      </div>

      {!loaded ? (
        <ListSkeleton variant="task" />
      ) : rooms.length === 0 ? (
        <EmptyState
          title="还没有房间"
          description="任务必须归属一个房间，请先创建房间。"
          action={
            <Link className="btn btn-primary btn-sm" to="/rooms">
              去创建房间
            </Link>
          }
        />
      ) : visibleTasks.length === 0 ? (
        <EmptyState
          title={roomFilter === null ? "还没有任务" : "该房间还没有任务"}
          description="点击右上角「创建任务」，选择房间后发布。"
          action={
            <button
              type="button"
              className="btn btn-primary btn-sm"
              onClick={openCreate}
            >
              创建任务
            </button>
          }
        />
      ) : view === "board" ? (
        <div className="task-board">
          {groupTasksByStatus(visibleTasks).map((column) => (
            <section
              key={column.status}
              className="task-board-column"
              aria-label={`${TASK_STATUS_LABELS[column.status]}列`}
            >
              <header className="task-board-column-head">
                <span>{TASK_STATUS_LABELS[column.status]}</span>
                <span className="muted small">{column.tasks.length}</span>
              </header>
              {column.tasks.length === 0 ? (
                <p className="muted small task-board-empty">暂无任务</p>
              ) : (
                column.tasks.map((task) => {
                  const due = formatDue(task.dueAt, Date.now());
                  const overdue =
                    task.dueAt !== null &&
                    task.dueAt < Date.now() &&
                    task.status !== "done" &&
                    task.status !== "cancelled";
                  const schedule = formatSchedule(
                    task.scheduledAt,
                    Date.now(),
                  );
                  return (
                    <article key={task.id} className="task-card">
                      <button
                        type="button"
                        className="task-card-title"
                        onClick={() => setSelectedTaskId(task.id)}
                      >
                        {task.title}
                      </button>
                      <div className="task-card-meta">
                        <span className={`priority-pill ${task.priority}`}>
                          {TASK_PRIORITY_LABELS[task.priority]}
                        </span>
                        <span className="muted small">
                          {roomNames.get(task.roomId) ??
                            `房间 #${task.roomId}`}
                        </span>
                        <span className="muted small">
                          {task.assigneeAgentId === null
                            ? "未指派"
                            : (agentNames.get(task.assigneeAgentId) ??
                              `#${task.assigneeAgentId}`)}
                        </span>
                        {due !== null && (
                          <span
                            className={overdue ? "task-due overdue" : "task-due"}
                          >
                            {due}
                          </span>
                        )}
                        {schedule !== null && (
                          <span className="task-schedule">{schedule}</span>
                        )}
                      </div>
                      <div className="task-row-actions">
                        {canActivateTask(task) && (
                          <button
                            type="button"
                            className="btn btn-secondary btn-sm"
                            disabled={
                              busyTaskId === task.id ||
                              activatingTaskId === task.id
                            }
                            onClick={() => void activate(task)}
                          >
                            {activatingTaskId === task.id
                              ? "启动中…"
                              : "启动编排"}
                          </button>
                        )}
                        <button
                          type="button"
                          className="btn btn-ghost btn-sm"
                          onClick={() => openEdit(task)}
                        >
                          编辑
                        </button>
                      </div>
                    </article>
                  );
                })
              )}
            </section>
          ))}
        </div>
      ) : (
        <div className="task-table-wrap">
          <table className="task-table">
            <thead>
              <tr>
                <th scope="col">任务</th>
                <th scope="col">房间</th>
                <th scope="col">状态</th>
                <th scope="col">优先级</th>
                <th scope="col">指派人</th>
                <th scope="col">截止</th>
                <th scope="col">更新时间</th>
                <th scope="col">操作</th>
              </tr>
            </thead>
            <tbody>
              {visibleTasks.map((task) => {
                const busy =
                  busyTaskId === task.id || activatingTaskId === task.id;
                const due = formatDue(task.dueAt, Date.now());
                const overdue =
                  task.dueAt !== null &&
                  task.dueAt < Date.now() &&
                  task.status !== "done" &&
                  task.status !== "cancelled";
                const schedule = formatSchedule(task.scheduledAt, Date.now());
                return (
                  <tr key={task.id}>
                    <td>
                      <div className="task-cell-main">
                        <div className="task-cell-title">
                          <span>{task.title}</span>
                          <span className={`priority-pill ${task.priority}`}>
                            {TASK_PRIORITY_LABELS[task.priority]}
                          </span>
                          {task.createdByAgentId !== null && (
                            <span className="badge badge-moderator">
                              主持人创建
                            </span>
                          )}
                          {schedule !== null && (
                            <span className="task-schedule">{schedule}</span>
                          )}
                        </div>
                        {task.description !== "" && (
                          <p className="task-cell-desc" title={task.description}>
                            {task.description}
                          </p>
                        )}
                      </div>
                    </td>
                    <td>
                      <Link
                        className="task-room-link"
                        to={`/rooms/${task.roomId}`}
                      >
                        {roomNames.get(task.roomId) ?? `房间 #${task.roomId}`}
                      </Link>
                    </td>
                    <td>
                      <span className={`status-pill ${task.status}`}>
                        {TASK_STATUS_LABELS[task.status]}
                      </span>
                    </td>
                    <td>
                      <span className={`priority-pill ${task.priority}`}>
                        {TASK_PRIORITY_LABELS[task.priority]}
                      </span>
                    </td>
                    <td className="task-assignee">
                      {task.assigneeAgentId === null ? (
                        <span className="muted small">未指派</span>
                      ) : (
                        (agentNames.get(task.assigneeAgentId) ??
                        `#${task.assigneeAgentId}`)
                      )}
                    </td>
                    <td className={overdue ? "task-due overdue" : "task-due"}>
                      {due ?? <span className="muted small">—</span>}
                    </td>
                    <td className="task-cell-time">
                      {formatTime(task.updatedAt)}
                    </td>
                    <td>
                      <div className="task-row-actions">
                        {canActivateTask(task) && (
                          <button
                            type="button"
                            className="btn btn-secondary btn-sm"
                            disabled={busy}
                            onClick={() => void activate(task)}
                          >
                            {activatingTaskId === task.id
                              ? "启动中…"
                              : "启动编排"}
                          </button>
                        )}
                        <button
                          type="button"
                          className="btn btn-ghost btn-sm"
                          onClick={() => setSelectedTaskId(task.id)}
                        >
                          详情
                        </button>
                        <button
                          type="button"
                          className="btn btn-ghost btn-sm"
                          onClick={() => openEdit(task)}
                        >
                          编辑
                        </button>
                      </div>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}

      {selectedTask !== null && (
        <TaskDetailPanel
          task={selectedTask}
          roomName={
            roomNames.get(selectedTask.roomId) ??
            `房间 #${selectedTask.roomId}`
          }
          assigneeName={
            selectedTask.assigneeAgentId === null
              ? null
              : (agentNames.get(selectedTask.assigneeAgentId) ??
                `#${selectedTask.assigneeAgentId}`)
          }
          actions={detailActions}
          onClose={() => setSelectedTaskId(null)}
        />
      )}

      {createOpen && (
        <div className="modal-layer">
          <div
            className="modal-card"
            role="dialog"
            aria-modal="true"
            aria-labelledby="task-form-title"
          >
            <header className="modal-head">
              <h2 id="task-form-title" className="card-title">
                {editingTaskId === null ? "创建任务" : "编辑任务"}
              </h2>
              <button
                type="button"
                className="btn btn-ghost btn-sm"
                aria-label="关闭创建任务"
                disabled={creating}
                onClick={closeCreate}
              >
                关闭
              </button>
            </header>

            {formError !== null && (
              <p className="banner error" role="alert">
                {formError}
              </p>
            )}

            <form className="form-stack" onSubmit={submitTask}>
              <label className="field">
                <span className="field-label">房间</span>
                <select
                  required
                  value={formRoomId}
                  onChange={(event) => {
                    setFormRoomId(event.target.value);
                    setFormAssigneeId("");
                  }}
                  disabled={creating || editingTaskId !== null}
                >
                  <option value="">选择房间</option>
                  {rooms.map((room) => (
                    <option key={room.id} value={room.id}>
                      {room.name}
                    </option>
                  ))}
                </select>
                {editingTaskId !== null && (
                  <span className="field-hint">任务归属房间不可修改。</span>
                )}
              </label>

              <label className="field">
                <span className="field-label">任务标题</span>
                <input
                  required
                  autoFocus
                  value={formTitle}
                  onChange={(event) => setFormTitle(event.target.value)}
                  placeholder="例如：实现登录页"
                  disabled={creating}
                />
              </label>

              <label className="field">
                <span className="field-label">描述（可选）</span>
                <textarea
                  rows={3}
                  value={formDescription}
                  onChange={(event) => setFormDescription(event.target.value)}
                  placeholder="补充验收要点或上下文"
                  disabled={creating}
                />
              </label>

              <div className="task-form-grid">
                <label className="field">
                  <span className="field-label">优先级</span>
                  <select
                    value={formPriority}
                    onChange={(event) =>
                      setFormPriority(event.target.value as TaskPriority)
                    }
                    disabled={creating}
                  >
                    {PRIORITY_OPTIONS.map((priority) => (
                      <option key={priority} value={priority}>
                        {TASK_PRIORITY_LABELS[priority]}
                      </option>
                    ))}
                  </select>
                </label>

                <label className="field">
                  <span className="field-label">指派人（可选）</span>
                  <select
                    value={formAssigneeId}
                    onChange={(event) => setFormAssigneeId(event.target.value)}
                    disabled={creating || roomMembers.length === 0}
                  >
                    <option value="">不指派</option>
                    {roomMembers.map((agent) => (
                      <option key={agent.id} value={agent.id}>
                        {agent.name}
                      </option>
                    ))}
                  </select>
                  {roomMembers.length === 0 && (
                    <span className="field-hint">
                      先选择房间；只有房间成员可被指派。
                    </span>
                  )}
                </label>

                <label className="field">
                  <span className="field-label">截止时间（可选）</span>
                  <input
                    type="datetime-local"
                    value={formDueAt}
                    onChange={(event) => setFormDueAt(event.target.value)}
                    disabled={creating}
                  />
                </label>

                <label className="field">
                  <span className="field-label">定时启动（可选）</span>
                  <input
                    type="datetime-local"
                    value={formScheduledAt}
                    onChange={(event) => setFormScheduledAt(event.target.value)}
                    disabled={creating || scheduleDisabled}
                  />
                  <span className="field-hint">
                    {scheduleDisabled
                      ? "只有待办任务可以设置定时启动。"
                      : "到点后自动转为进行中；创建时留空表示不定时。"}
                  </span>
                </label>
              </div>

              <div className="modal-actions">
                <button
                  type="button"
                  className="btn btn-secondary"
                  disabled={creating}
                  onClick={closeCreate}
                >
                  取消
                </button>
                <button
                  type="submit"
                  className="btn btn-primary"
                  disabled={
                    creating ||
                    formRoomId === "" ||
                    formTitle.trim() === ""
                  }
                >
                  {creating
                    ? "保存中…"
                    : editingTaskId === null
                      ? "创建任务"
                      : "保存修改"}
                </button>
              </div>
            </form>
          </div>
        </div>
      )}
    </section>
  );
}
