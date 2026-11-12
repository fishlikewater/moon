import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type FormEvent,
} from "react";
import { Link, useSearchParams } from "react-router-dom";
import type {
  Agent,
  Room,
  RoomPermissionMode,
  RoomReplyMode,
} from "@moon/shared";
import { api, errorMessage, ROOM_DISCUSSION_ROUNDS } from "../api/client";
import { AgentAvatar } from "../components/AgentAvatar";
import { EmptyState } from "../components/layout/EmptyState";
import { ListSkeleton } from "../components/layout/ListSkeleton";
import { PageHeader } from "../components/layout/PageHeader";
import { shouldOpenNew, withoutNewParam } from "./newEntry";
import {
  avatarStack,
  formatActivity,
  visibleRooms,
  type RoomSort,
} from "./roomList";

const PERMISSION_MODE_OPTIONS: ReadonlyArray<{
  value: RoomPermissionMode;
  label: string;
  hint: string;
}> = [
  {
    value: "plan",
    label: "计划模式",
    hint: "只读规划：不写文件、不执行有副作用的命令。",
  },
  {
    value: "auto",
    label: "自主模式",
    hint: "可在工作区读写与执行，危险操作由 harness 拦截（新房间默认）。",
  },
  {
    value: "full",
    label: "完全访问",
    hint: "无审批、无沙箱；仅在可信工作区使用。",
  },
];

const REPLY_MODE_OPTIONS: ReadonlyArray<{
  value: RoomReplyMode;
  label: string;
  hint: string;
}> = [
  {
    value: "moderator",
    label: "主持人自动接管",
    hint: "未 @ 任何成员时，由内置主持人判断直接回答、委派或创建任务。",
  },
  {
    value: "mentions",
    label: "仅 @ 成员回复",
    hint: "只有被 @ 的成员会运行；主持人仅在被显式 @ 时回复。",
  },
];

interface RoomFormState {
  editingId: number | null;
  name: string;
  workspaceDirs: string[];
  workspaceInput: string;
  memberIds: number[];
  replyMode: RoomReplyMode;
  discussionMaxRounds: string;
  permissionMode: RoomPermissionMode;
}

const EMPTY_FORM: RoomFormState = {
  editingId: null,
  name: "",
  workspaceDirs: [],
  workspaceInput: "",
  memberIds: [],
  replyMode: "moderator",
  discussionMaxRounds: String(ROOM_DISCUSSION_ROUNDS.fallback),
  permissionMode: "auto",
};

/** 桌面端由 Tauri 注入；浏览器预览时退回手动输入目录。 */
const IS_TAURI =
  typeof window !== "undefined" &&
  (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ !==
    undefined;

function mergeWorkspaceDirs(current: string[], next: string[]): string[] {
  const merged = [...current];
  for (const dir of next) {
    const trimmed = dir.trim();
    if (trimmed !== "" && !merged.includes(trimmed)) {
      merged.push(trimmed);
    }
  }
  return merged;
}

/**
 * 房间管理页：创建/编辑房间、维护成员与回复模式。
 *
 * 主持人固定为系统内置智能体，页面不提供选择或更换入口；
 * 成员增删与回复模式变更走独立接口并立即回读服务端结果。
 */
export default function RoomsPage() {
  const [rooms, setRooms] = useState<Room[]>([]);
  const [agents, setAgents] = useState<Agent[]>([]);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [form, setForm] = useState<RoomFormState>(EMPTY_FORM);
  const [formOpen, setFormOpen] = useState(false);
  const [roomDefaults, setRoomDefaults] = useState<{
    permissionMode: RoomPermissionMode;
    discussionMaxRounds: string;
  }>({
    permissionMode: EMPTY_FORM.permissionMode,
    discussionMaxRounds: EMPTY_FORM.discussionMaxRounds,
  });
  const [searchParams, setSearchParams] = useSearchParams();
  const [deletingRoomId, setDeletingRoomId] = useState<number | null>(null);
  const [roomQuery, setRoomQuery] = useState("");
  const [roomSort, setRoomSort] = useState<RoomSort>("activity");
  const modalRef = useRef<HTMLDivElement>(null);
  const nameInputRef = useRef<HTMLInputElement>(null);

  const reload = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [agentList, roomList, settingsResponse] = await Promise.all([
        api.agents.list(),
        api.rooms.list(),
        api.settings.get().catch((cause: unknown) => {
          console.warn("读取设置失败，使用默认建房值", cause);
          return null;
        }),
      ]);
      setAgents(agentList);
      setRooms(roomList);
      if (settingsResponse !== null) {
        setRoomDefaults({
          permissionMode:
            settingsResponse.settings.defaultRoomPermissionMode,
          discussionMaxRounds: String(
            settingsResponse.settings.defaultRoomDiscussionRounds,
          ),
        });
      }
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  // 打开时锁定页面滚动并把焦点移入表单，关闭后把焦点还给触发按钮。
  useEffect(() => {
    if (!formOpen) {
      return;
    }
    const previouslyFocused =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    nameInputRef.current?.focus();

    return () => {
      document.body.style.overflow = previousOverflow;
      previouslyFocused?.focus();
    };
  }, [formOpen]);

  // Esc 关闭（保存中禁用），Tab 在对话框内循环。
  useEffect(() => {
    if (!formOpen) {
      return;
    }
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        if (!busy) {
          setFormOpen(false);
          setForm(EMPTY_FORM);
        }
        return;
      }
      if (event.key !== "Tab") {
        return;
      }
      const focusable = modalRef.current?.querySelectorAll<HTMLElement>(
        'button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), a[href]',
      );
      if (focusable === undefined || focusable.length === 0) {
        return;
      }
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    };
    document.addEventListener("keydown", handleKeyDown);

    return () => {
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [formOpen, busy]);

  const nameOf = (agentId: number): string =>
    agents.find((agent) => agent.id === agentId)?.name ?? `#${agentId}`;

  const builtinModerator =
    agents.find((agent) => agent.builtinRole === "moderator") ?? null;
  // 内置主持人由系统自动加入，不出现在可选成员列表中。
  const userAgents = agents.filter(
    (agent) => agent.builtinRole !== "moderator",
  );

  const toggleMember = (agentId: number) => {
    setForm((current) => ({
      ...current,
      memberIds: current.memberIds.includes(agentId)
        ? current.memberIds.filter((id) => id !== agentId)
        : [...current.memberIds, agentId],
    }));
  };

  const addWorkspaceDirs = (dirs: string[]) => {
    setForm((current) => ({
      ...current,
      workspaceDirs: mergeWorkspaceDirs(current.workspaceDirs, dirs),
    }));
  };

  const addWorkspaceInput = () => {
    const value = form.workspaceInput.trim();
    if (value === "") {
      return;
    }
    addWorkspaceDirs([value]);
    setForm((current) => ({ ...current, workspaceInput: "" }));
  };

  const removeWorkspaceDir = (dir: string) => {
    setForm((current) => ({
      ...current,
      workspaceDirs: current.workspaceDirs.filter((item) => item !== dir),
    }));
  };

  const pickWorkspaceDirs = async () => {
    if (!IS_TAURI) {
      setError("浏览器预览不支持系统目录选择，请手动输入绝对路径。");
      return;
    }
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const picked = await open({
        directory: true,
        multiple: true,
        title: "选择房间工作空间目录",
      });
      const dirs = Array.isArray(picked) ? picked : picked ? [picked] : [];
      addWorkspaceDirs(dirs);
    } catch (cause) {
      setError(errorMessage(cause));
    }
  };

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const workspaceDirs = form.workspaceDirs
      .map((dir) => dir.trim())
      .filter((dir) => dir !== "");
    if (workspaceDirs.length === 0) {
      setError("请至少选择一个工作空间目录。");
      return;
    }
    setBusy(true);
    setError(null);

    try {
      if (form.editingId === null) {
        await api.rooms.create({
          name: form.name.trim(),
          workspaceDirs,
          memberIds: form.memberIds,
          replyMode: form.replyMode,
          discussionMaxRounds: Number(form.discussionMaxRounds),
          permissionMode: form.permissionMode,
        });
      } else {
        const editingId = form.editingId;
        await api.rooms.update(editingId, {
          name: form.name.trim(),
          workspaceDirs,
          replyMode: form.replyMode,
          discussionMaxRounds: Number(form.discussionMaxRounds),
          permissionMode: form.permissionMode,
        });
        // 成员增删走既有 add/remove 端点（update 不含成员），差异提交；
        // 内置主持人由服务端固定，不在 userAgents 里，不会被移除。
        const current = rooms.find((room) => room.id === editingId);
        if (current !== undefined) {
          const originalIds = new Set(current.memberIds);
          const nextIds = new Set(form.memberIds);
          const moderatorId =
            current.moderatorAgentId ?? builtinModerator?.id ?? null;
          const removed = [...originalIds].filter(
            (id) => !nextIds.has(id) && id !== moderatorId,
          );
          const added = [...nextIds].filter((id) => !originalIds.has(id));
          for (const agentId of removed) {
            await api.rooms.removeMember(editingId, agentId);
          }
          for (const agentId of added) {
            await api.rooms.addMember(editingId, agentId);
          }
        }
      }
      setFormOpen(false);
      setForm(EMPTY_FORM);
      await reload();
    } catch (cause) {
      const message = errorMessage(cause);
      // 成员差异可能已部分提交：先回读服务端状态，避免基于旧成员集合重试再次失败。
      await reload();
      setError(message);
    } finally {
      setBusy(false);
    }
  };

  const startCreate = () => {
    setError(null);
    setForm({
      ...EMPTY_FORM,
      permissionMode: roomDefaults.permissionMode,
      discussionMaxRounds: roomDefaults.discussionMaxRounds,
    });
    setFormOpen(true);
  };

  // 命令面板「新建房间」入口：打开一次后从地址栏移除参数，避免刷新重复弹出。
  useEffect(() => {
    if (loading || !shouldOpenNew(searchParams)) {
      return;
    }
    startCreate();
    setSearchParams(withoutNewParam(searchParams), { replace: true });
    // startCreate 依赖最新 roomDefaults；参数清理后不会再次触发。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loading, searchParams]);

  const startEdit = (room: Room) => {
    setError(null);
    setForm({
      editingId: room.id,
      name: room.name,
      workspaceDirs:
        room.workspaceDirs.length > 0
          ? room.workspaceDirs
          : [room.workspaceDir],
      workspaceInput: "",
      memberIds: room.memberIds,
      replyMode: room.replyMode,
      discussionMaxRounds: String(room.discussionMaxRounds),
      permissionMode: room.permissionMode,
    });
    setFormOpen(true);
  };

  const closeForm = () => {
    if (busy) {
      return;
    }
    setFormOpen(false);
    setForm(EMPTY_FORM);
  };

  const removeRoom = async (room: Room) => {
    if (deletingRoomId !== null) {
      return;
    }
    if (
      !window.confirm(
        `确认删除房间「${room.name}」？房间内的消息、任务与讨论记录会一并删除。`,
      )
    ) {
      return;
    }
    setError(null);
    setDeletingRoomId(room.id);
    try {
      await api.rooms.remove(room.id);
      setFormOpen(false);
      setForm(EMPTY_FORM);
      await reload();
    } catch (cause) {
      const message = errorMessage(cause);
      // reload 入口会清空 error，因此回读列表后再写入失败提示。
      await reload();
      setError(message);
    } finally {
      setDeletingRoomId(null);
    }
  };

  // 房间数量有限，每次渲染直接过滤排序即可；搜索/排序组合保持纯函数可测。
  const orderedPage = visibleRooms(rooms, roomQuery, roomSort);

  return (
    <section className="page">
      <PageHeader
        title="房间"
        subtitle="绑定工作空间目录、选择成员与回复模式；内置主持人自动加入。"
        actions={
          <>
            <div className="page-stat">
              <span className="page-stat-value">{rooms.length}</span>
              <span className="page-stat-label">房间</span>
            </div>
            <button
              type="button"
              className="btn btn-primary"
              onClick={startCreate}
            >
              新建房间
            </button>
          </>
        }
      />

      {error !== null && !formOpen && (
        <p className="banner error" role="alert">
          {error}
        </p>
      )}

      <div className="manage-main">
          {loading ? (
            <ListSkeleton variant="room" />
          ) : rooms.length === 0 ? (
            <EmptyState
              title="还没有房间，先创建一个。"
              action={
                <button
                  type="button"
                  className="btn btn-primary btn-sm"
                  onClick={startCreate}
                >
                  新建房间
                </button>
              }
            />
          ) : (
            <>
              <div className="room-toolbar">
                <input
                  type="search"
                  className="input room-search"
                  aria-label="搜索房间"
                  placeholder="搜索房间名称或目录…"
                  value={roomQuery}
                  onChange={(event) => setRoomQuery(event.target.value)}
                />
                <label className="room-sort-field">
                  <span className="field-label">排序</span>
                  <select
                    value={roomSort}
                    aria-label="房间排序"
                    onChange={(event) =>
                      setRoomSort(event.target.value as RoomSort)
                    }
                  >
                    <option value="activity">最近活跃</option>
                    <option value="name">名称</option>
                  </select>
                </label>
                <span className="muted small room-count">
                  共 {orderedPage.length} 个房间
                </span>
              </div>
              {orderedPage.length === 0 ? (
                <EmptyState
                  title="没有匹配的房间"
                  action={
                    <button
                      type="button"
                      className="btn btn-secondary btn-sm"
                      onClick={() => setRoomQuery("")}
                    >
                      清空搜索
                    </button>
                  }
                />
              ) : (
                <ul className="room-list" aria-label="房间列表">
                  {orderedPage.map((room) => {
                    const memberNames = room.memberIds
                      .map((agentId) => {
                        const agent = agents.find((item) => item.id === agentId);
                        return {
                          id: agentId,
                          name: agent?.name ?? `#${agentId}`,
                          builtinRole: agent?.builtinRole ?? null,
                        };
                      })
                      .sort(
                        (left, right) =>
                          Number(right.builtinRole === "moderator") -
                          Number(left.builtinRole === "moderator"),
                      );
                    const stack = avatarStack(room.memberIds);
                    const activityLabel = formatActivity(
                      room.lastActivityAt,
                      Date.now(),
                    );
                    const activityTitle =
                      room.lastActivityAt === null
                        ? "暂无消息"
                        : new Date(room.lastActivityAt).toLocaleString("zh-CN");
                    return (
                      <li key={room.id} className="room-row">
                        <div className="room-row-main">
                          <div className="room-identity">
                            <h3 className="room-title">{room.name}</h3>
                            <p
                              className="path"
                              title={room.workspaceDirs.join("\n")}
                            >
                              {room.workspaceDir}
                              {room.workspaceDirs.length > 1
                                ? ` · 另有 ${room.workspaceDirs.length - 1} 个目录`
                                : ""}
                            </p>
                          </div>
                          <span className="room-avatar-stack" aria-hidden="true">
                            {stack.shown.map((agentId) => {
                              const agent = agents.find(
                                (item) => item.id === agentId,
                              );
                              return (
                                <AgentAvatar
                                  key={agentId}
                                  id={agentId}
                                  name={agent?.name ?? `#${agentId}`}
                                  size="sm"
                                />
                              );
                            })}
                            {stack.overflow > 0 && (
                              <span className="avatar-overflow">
                                +{stack.overflow}
                              </span>
                            )}
                          </span>
                          <span className="muted small room-member-count">
                            {memberNames.length} 名成员
                          </span>
                          <time className="room-activity" title={activityTitle}>
                            {activityLabel}
                          </time>
                          <div className="room-row-actions">
                            <Link
                              className="btn btn-primary btn-sm"
                              to={`/rooms/${room.id}`}
                            >
                              进入
                            </Link>
                            <button
                              type="button"
                              className="btn btn-ghost btn-sm"
                              aria-label={`编辑房间 ${room.name}`}
                              onClick={() => startEdit(room)}
                            >
                              编辑
                            </button>
                            <button
                              type="button"
                              className="btn btn-ghost btn-sm danger-text"
                              aria-label={`删除房间 ${room.name}`}
                              disabled={deletingRoomId !== null}
                              onClick={() => void removeRoom(room)}
                            >
                              {deletingRoomId === room.id ? "删除中…" : "删除"}
                            </button>
                          </div>
                        </div>
                        <div className="room-row-meta">
                          <span className="badge badge-moderator">
                            主持人：
                            {builtinModerator?.name ??
                              (room.moderatorAgentId === null
                                ? "未配置"
                                : nameOf(room.moderatorAgentId))}
                          </span>
                          <span className="badge badge-reply-mode">
                            {room.replyMode === "moderator"
                              ? "主持人自动接管"
                              : "仅 @ 成员回复"}
                          </span>
                          <span className="badge badge-reply-mode">
                            讨论上限：{room.discussionMaxRounds} 轮
                          </span>
                          <span className="badge badge-reply-mode">
                            {PERMISSION_MODE_OPTIONS.find(
                              (option) => option.value === room.permissionMode,
                            )?.label ?? room.permissionMode}
                          </span>
                        </div>
                      </li>
                    );
                  })}
                </ul>
              )}
            </>
          )}
      </div>

      {formOpen && (
        <div className="modal-layer">
          <div
            className="modal-card"
            ref={modalRef}
            role="dialog"
            aria-modal="true"
            aria-labelledby="room-form-title"
          >
            <header className="modal-head">
              <h2 id="room-form-title" className="card-title">
                {form.editingId === null ? "新建房间" : "编辑房间"}
              </h2>
              <button
                type="button"
                className="btn btn-ghost btn-sm"
                aria-label="关闭房间表单"
                onClick={closeForm}
                disabled={busy}
              >
                关闭
              </button>
            </header>
            {error !== null && (
              <p className="banner error" role="alert">
                {error}
              </p>
            )}
            <form className="form-stack" onSubmit={submit}>

            <label className="field">
              <span className="field-label">名称</span>
              <input
                required
                ref={nameInputRef}
                value={form.name}
                onChange={(event) =>
                  setForm({ ...form, name: event.target.value })
                }
                placeholder="例如：设计讨论间"
              />
            </label>

            <fieldset className="workspace-field">
              <legend>工作空间目录</legend>
              {form.workspaceDirs.length === 0 ? (
                <p className="muted small">尚未选择目录，至少需要一个。</p>
              ) : (
                <ul className="workspace-list">
                  {form.workspaceDirs.map((dir, index) => (
                    <li key={dir} className="workspace-item">
                      <span className="workspace-role">
                        {index === 0 ? "主目录" : "附加"}
                      </span>
                      <span className="workspace-path" title={dir}>
                        {dir}
                      </span>
                      <button
                        type="button"
                        className="btn btn-ghost btn-sm"
                        onClick={() => removeWorkspaceDir(dir)}
                        disabled={busy}
                      >
                        移除
                      </button>
                    </li>
                  ))}
                </ul>
              )}
              <div className="workspace-actions">
                <button
                  type="button"
                  className="btn btn-secondary btn-sm"
                  onClick={() => void pickWorkspaceDirs()}
                  disabled={busy}
                >
                  选择目录…
                </button>
                <input
                  className="input"
                  value={form.workspaceInput}
                  onChange={(event) =>
                    setForm({ ...form, workspaceInput: event.target.value })
                  }
                  onKeyDown={(event) => {
                    if (event.key === "Enter") {
                      event.preventDefault();
                      addWorkspaceInput();
                    }
                  }}
                  placeholder={
                    IS_TAURI
                      ? "或手动输入绝对路径"
                      : "输入本机绝对路径，例如 E:\\workspace\\demo"
                  }
                />
                <button
                  type="button"
                  className="btn btn-secondary btn-sm"
                  onClick={addWorkspaceInput}
                  disabled={busy || form.workspaceInput.trim() === ""}
                >
                  添加
                </button>
              </div>
              <p className="field-hint">
                第一个目录作为智能体运行目录，其余目录作为附加目录传给 harness；
                至少一个，最多 32 个。
              </p>
            </fieldset>

            <label className="field">
              <span className="field-label">讨论轮数上限</span>
              <input
                type="number"
                required
                min={ROOM_DISCUSSION_ROUNDS.min}
                max={ROOM_DISCUSSION_ROUNDS.max}
                step={1}
                value={form.discussionMaxRounds}
                onChange={(event) =>
                  setForm({
                    ...form,
                    discussionMaxRounds: event.target.value,
                  })
                }
              />
              <span className="field-hint">
                主持人发起讨论时可在此范围内选择轮数，默认{" "}
                {ROOM_DISCUSSION_ROUNDS.fallback} 轮；修改只影响之后新发起的讨论。
              </span>
            </label>

            <fieldset className="member-picker-field">
              <legend>成员</legend>
              {userAgents.length === 0 ? (
                <p className="muted small">
                  还没有可选的智能体，请先到「智能体」页创建。
                </p>
              ) : (
                <div className="member-picker">
                  {userAgents.map((agent) => {
                    const checked = form.memberIds.includes(agent.id);
                    return (
                      <label
                        key={agent.id}
                        className={checked ? "chip-check checked" : "chip-check"}
                      >
                        <input
                          type="checkbox"
                          checked={checked}
                          onChange={() => toggleMember(agent.id)}
                        />
                        <span>{agent.name}</span>
                      </label>
                    );
                  })}
                </div>
              )}
              <p className="field-hint">
                内置主持人「{builtinModerator?.name ?? "Moon 主持人"}」会自动加入并担任主持人，不能在成员列表中移除或更换。
              </p>
            </fieldset>

            <fieldset className="reply-mode-field">
              <legend>回复模式</legend>
              {REPLY_MODE_OPTIONS.map((option) => (
                <label key={option.value} className="reply-mode-option">
                  <input
                    type="radio"
                    name="replyMode"
                    value={option.value}
                    checked={form.replyMode === option.value}
                    onChange={() =>
                      setForm({ ...form, replyMode: option.value })
                    }
                  />
                  <span>
                    <span className="reply-mode-label">{option.label}</span>
                    <span className="field-hint">{option.hint}</span>
                  </span>
                </label>
              ))}
            </fieldset>

            <fieldset className="reply-mode-field">
              <legend>权限模式</legend>
              {PERMISSION_MODE_OPTIONS.map((option) => (
                <label key={option.value} className="reply-mode-option">
                  <input
                    type="radio"
                    name="permissionMode"
                    value={option.value}
                    checked={form.permissionMode === option.value}
                    onChange={() =>
                      setForm({ ...form, permissionMode: option.value })
                    }
                  />
                  <span>
                    <span className="reply-mode-label">{option.label}</span>
                    <span className="field-hint">{option.hint}</span>
                  </span>
                </label>
              ))}
            </fieldset>

            {form.permissionMode === "full" && (
              <p className="field-hint permission-warning" role="alert">
                完全访问会跳过审批与文件系统沙箱，智能体可以直接修改或删除工作区文件；仅在可信目录使用。
              </p>
            )}

            <div className="form-actions">
              <button type="submit" className="btn btn-primary" disabled={busy}>
                {busy
                  ? "保存中…"
                  : form.editingId === null
                    ? "创建房间"
                    : "保存修改"}
              </button>
              <button
                type="button"
                className="btn btn-secondary"
                onClick={closeForm}
                disabled={busy}
              >
                取消
              </button>
            </div>
            </form>
          </div>
        </div>
      )}
    </section>
  );
}
