import type {
  Agent,
  AgentBuilderChatRequest,
  AgentBuilderChatResponse,
  AgentRun,
  AgentSession,
  AcpProbeResponse,
  AppSettings,
  AppSettingsResponse,
  CollaborationBatch,
  ContextPreview,
  CustomHarness,
  CustomHarnessCreateInput,
  CustomHarnessUpdateInput,
  Discussion,
  HarnessKind,
  HarnessModelCatalog,
  Message,
  MessageAttachment,
  MessageCleanupResult,
  MessageReaction,
  ModelConfig,
  QueueSnapshot,
  Room,
  RoomFiles,
  RoomMemberSessionSettings,
  RoomPermissionMode,
  RoomReplyMode,
  RoomSessionsResponse,
  RunDetailResponse,
  SessionOverview,
  Task,
  TaskPriority,
  TaskStatus,
  TaskTimelineResponse,
} from "@moon/shared";
import type { AcpRegistryPayload } from "../acpRegistryCatalog";
import { apiUrl } from "../runtime";

/**
 * 前端 API client。
 *
 * 服务端错误统一为 `{ error: { code, message } }`，这里解析后抛 `ApiError`，
 * 页面只负责展示 message 与按 code 做差异化处理，不各自解析响应。
 */
export interface AgentCreateInput {
  name: string;
  persona: string;
  harness: HarnessKind;
  modelConfig?: ModelConfig | null;
  /** harness 为 acp 时必填：指向自定义 ACP agent 注册表条目。 */
  customHarnessId?: number | null;
  /** 独立工作区；null 或空串表示使用房间主工作区。 */
  workspaceDir?: string | null;
}

export type AgentUpdateInput = Partial<AgentCreateInput>;

export interface RoomCreateInput {
  name: string;
  /** 至少一个目录；第一个作为运行 cwd，其余作为附加目录。 */
  workspaceDirs: string[];
  memberIds?: number[];
  replyMode?: RoomReplyMode;
  discussionMaxRounds?: number;
  permissionMode?: RoomPermissionMode;
}

export type RoomUpdateInput = Partial<
  Pick<
    RoomCreateInput,
    "name" | "workspaceDirs" | "replyMode" | "discussionMaxRounds" | "permissionMode"
  >
>;

/**
 * 房间讨论轮数边界的本地副本。
 *
 * shared 包是 CommonJS 产物，web 构建不加载其运行时；这三个数值必须与
 * shared 的 MIN/MAX/DEFAULT_ROOM_DISCUSSION_ROUNDS 保持一致，服务端会再次校验。
 */
export const ROOM_DISCUSSION_ROUNDS = { min: 1, max: 50, fallback: 5 } as const;

export interface TaskCreateInput {
  title: string;
  description?: string;
  priority?: TaskPriority;
  /** 指派的房间成员；null 表示不指派。 */
  assigneeAgentId?: number | null;
  /** 截止时间（epoch 毫秒）；null 表示不设置。 */
  dueAt?: number | null;
  /** 定时自动启动时间（epoch 毫秒）；仅 backlog 可设置。 */
  scheduledAt?: number | null;
}

/** 任务可编辑字段补丁；省略的键保持不变，显式 null 清除可空字段。 */
export interface TaskUpdateInput {
  title?: string;
  description?: string;
  priority?: TaskPriority;
  assigneeAgentId?: number | null;
  dueAt?: number | null;
  scheduledAt?: number | null;
}

/** 房间级暂停开关状态；`paused` 为服务端当前值（幂等接口）。 */
export interface OrchestrationPauseState {
  roomId: number;
  paused: boolean;
}

export class ApiError extends Error {
  readonly code: string;
  readonly status: number;

  constructor(
    status: number,
    code: string,
    message: string,
    options: { cause?: unknown } = {},
  ) {
    super(message, { cause: options.cause });
    this.name = "ApiError";
    this.code = code;
    this.status = status;
  }
}

interface ErrorBody {
  error: { code: string; message: string };
}

function isErrorBody(value: unknown): value is ErrorBody {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  const error = (value as { error?: unknown }).error;
  if (typeof error !== "object" || error === null) {
    return false;
  }
  const candidate = error as { code?: unknown; message?: unknown };
  return (
    typeof candidate.code === "string" && typeof candidate.message === "string"
  );
}

async function requestJson<T>(
  path: string,
  init: RequestInit = {},
): Promise<T> {
  const headers = new Headers(init.headers);
  const isFormData =
    typeof FormData !== "undefined" && init.body instanceof FormData;
  if (init.body !== undefined && !isFormData && !headers.has("Content-Type")) {
    headers.set("Content-Type", "application/json");
  }

  let response: Response;
  try {
    response = await fetch(apiUrl(path), { ...init, headers });
  } catch (cause) {
    throw new ApiError(
      0,
      "NETWORK_ERROR",
      "无法连接后端服务，请确认 API 已启动",
      { cause },
    );
  }

  if (response.status === 204) {
    return undefined as T;
  }

  // 响应体固定按 UTF-8 解析（fetch 规范的 JSON 解码行为）。
  const body: unknown = await response.json().catch(() => null);

  if (!response.ok) {
    const error = isErrorBody(body)
      ? body.error
      : {
          code: "HTTP_ERROR",
          message: `请求失败（HTTP ${response.status}）`,
        };
    throw new ApiError(response.status, error.code, error.message);
  }

  return body as T;
}

export const api = {
  settings: {
    get: (): Promise<AppSettingsResponse> =>
      requestJson<AppSettingsResponse>("/api/settings"),
    update: (patch: Partial<AppSettings>): Promise<AppSettings> =>
      requestJson<AppSettings>("/api/settings", {
        method: "PATCH",
        body: JSON.stringify(patch),
      }),
  },
  agents: {
    list: (): Promise<Agent[]> => requestJson<Agent[]>("/api/agents"),
    create: (input: AgentCreateInput): Promise<Agent> =>
      requestJson<Agent>("/api/agents", {
        method: "POST",
        body: JSON.stringify(input),
      }),
    update: (id: number, input: AgentUpdateInput): Promise<Agent> =>
      requestJson<Agent>(`/api/agents/${id}`, {
        method: "PATCH",
        body: JSON.stringify(input),
      }),
    remove: (id: number): Promise<void> =>
      requestJson<void>(`/api/agents/${id}`, { method: "DELETE" }),
  },
  agentBuilder: {
    chat: (
      input: AgentBuilderChatRequest,
    ): Promise<AgentBuilderChatResponse> =>
      requestJson<AgentBuilderChatResponse>("/api/agent-builder/chat", {
        method: "POST",
        body: JSON.stringify(input),
      }),
  },
  harnesses: {
    models: (harness: HarnessKind): Promise<HarnessModelCatalog> =>
      requestJson<HarnessModelCatalog>(
        `/api/harnesses/${encodeURIComponent(harness)}/models`,
      ),
  },
  acpRegistry: {
    list: (): Promise<AcpRegistryPayload> =>
      requestJson<AcpRegistryPayload>("/api/acp-registry"),
  },
  customHarnesses: {
    list: (): Promise<CustomHarness[]> =>
      requestJson<CustomHarness[]>("/api/custom-harnesses"),
    get: (id: number): Promise<CustomHarness> =>
      requestJson<CustomHarness>(`/api/custom-harnesses/${id}`),
    create: (input: CustomHarnessCreateInput): Promise<CustomHarness> =>
      requestJson<CustomHarness>("/api/custom-harnesses", {
        method: "POST",
        body: JSON.stringify(input),
      }),
    update: (id: number, input: CustomHarnessUpdateInput): Promise<CustomHarness> =>
      requestJson<CustomHarness>(`/api/custom-harnesses/${id}`, {
        method: "PATCH",
        body: JSON.stringify(input),
      }),
    remove: (id: number): Promise<void> =>
      requestJson<void>(`/api/custom-harnesses/${id}`, { method: "DELETE" }),
    probe: (id: number): Promise<AcpProbeResponse> =>
      requestJson<AcpProbeResponse>(`/api/custom-harnesses/${id}/probe`, {
        method: "POST",
      }),
  },
  rooms: {
    list: (): Promise<Room[]> => requestJson<Room[]>("/api/rooms"),
    create: (input: RoomCreateInput): Promise<Room> =>
      requestJson<Room>("/api/rooms", {
        method: "POST",
        body: JSON.stringify(input),
      }),
    update: (id: number, input: RoomUpdateInput): Promise<Room> =>
      requestJson<Room>(`/api/rooms/${id}`, {
        method: "PATCH",
        body: JSON.stringify(input),
      }),
    remove: (id: number): Promise<void> =>
      requestJson<void>(`/api/rooms/${id}`, { method: "DELETE" }),
    /** 发送前上下文估算；与真实运行共用压缩逻辑。 */
    contextPreview: (
      id: number,
      agentId: number,
      instruction = "",
    ): Promise<ContextPreview> => {
      const query = new URLSearchParams({ agentId: String(agentId) });
      if (instruction !== "") {
        query.set("instruction", instruction);
      }
      return requestJson<ContextPreview>(
        `/api/rooms/${id}/context?${query.toString()}`,
      );
    },
    addMember: (id: number, agentId: number): Promise<Room> =>
      requestJson<Room>(`/api/rooms/${id}/members`, {
        method: "POST",
        body: JSON.stringify({ agentId }),
      }),
    removeMember: (id: number, agentId: number): Promise<Room> =>
      requestJson<Room>(`/api/rooms/${id}/members/${agentId}`, {
        method: "DELETE",
      }),
    /** 当前运行与排队任务快照；未开跑的聊天/主持人任务会在重启后恢复。 */
    queue: (id: number): Promise<QueueSnapshot> =>
      requestJson<QueueSnapshot>(`/api/rooms/${id}/queue`),
    /** 暂停/恢复房间队列；运行中的任务不中断，后续任务等待恢复。 */
    setQueuePaused: (id: number, paused: boolean): Promise<QueueSnapshot> =>
      requestJson<QueueSnapshot>(`/api/rooms/${id}/queue/pause`, {
        method: "PUT",
        body: JSON.stringify({ paused }),
      }),
    /** 取消仍在排队的普通聊天；运行中/不可取消返回 409。 */
    cancelQueueJob: (id: number, jobId: number): Promise<void> =>
      requestJson<void>(`/api/rooms/${id}/queue/${jobId}`, {
        method: "DELETE",
      }),
    /** 房间与成员智能体工作区的只读文件清单。 */
    files: (id: number): Promise<RoomFiles> =>
      requestJson<RoomFiles>(`/api/rooms/${id}/files`),
  },
  sessions: {
    /** 全部房间的会话总览（含已归档），带房间名与智能体名。 */
    listAll: (): Promise<SessionOverview[]> =>
      requestJson<SessionOverview[]>("/api/sessions"),
    /** 房间会话列表与成员复用设置；含已归档会话。 */
    list: (roomId: number): Promise<RoomSessionsResponse> =>
      requestJson<RoomSessionsResponse>(`/api/rooms/${roomId}/sessions`),
    /** 设置成员是否复用会话；非成员返回 404。 */
    setReuse: (
      roomId: number,
      agentId: number,
      reuseSession: boolean,
    ): Promise<RoomMemberSessionSettings> =>
      requestJson<RoomMemberSessionSettings>(
        `/api/rooms/${roomId}/members/${agentId}/session-settings`,
        {
          method: "PUT",
          body: JSON.stringify({ reuseSession }),
        },
      ),
    /** 归档：保留存储，下次运行新建会话；房间忙时 409。 */
    archive: (roomId: number, sessionId: number): Promise<AgentSession> =>
      requestJson<AgentSession>(
        `/api/rooms/${roomId}/sessions/${sessionId}/archive`,
        { method: "POST" },
      ),
    /** 删除：记录与本地存储一起移除；房间忙时 409。 */
    remove: (roomId: number, sessionId: number): Promise<void> =>
      requestJson<void>(`/api/rooms/${roomId}/sessions/${sessionId}`, {
        method: "DELETE",
      }),
  },
  messages: {
    list: (roomId: number): Promise<Message[]> =>
      requestJson<Message[]>(`/api/rooms/${roomId}/messages`),
    /** 202：用户消息已接受，智能体回复经 WebSocket 推送。 */
    send: (
      roomId: number,
      content: string,
      attachmentIds: number[] = [],
      replyToId: number | null = null,
    ): Promise<{ userMessage: Message }> =>
      requestJson<{ userMessage: Message }>(`/api/rooms/${roomId}/messages`, {
        method: "POST",
        body: JSON.stringify({
          content,
          ...(attachmentIds.length === 0 ? {} : { attachmentIds }),
          ...(replyToId === null ? {} : { replyToId }),
        }),
      }),
    /** 切换固定调色板内的表情反应，返回该消息的最新反应。 */
    toggleReaction: (
      roomId: number,
      messageId: number,
      emoji: string,
    ): Promise<{ reactions: MessageReaction[] }> =>
      requestJson<{ reactions: MessageReaction[] }>(
        `/api/rooms/${roomId}/messages/${messageId}/reactions`,
        {
          method: "POST",
          body: JSON.stringify({ emoji }),
        },
      ),
    /** 清理纯聊天历史；keepRecent=0 表示全部清理。 */
    cleanup: (
      roomId: number,
      keepRecent: number,
    ): Promise<MessageCleanupResult> =>
      requestJson<MessageCleanupResult>(
        `/api/rooms/${roomId}/messages/cleanup`,
        {
          method: "POST",
          body: JSON.stringify({ keepRecent }),
        },
      ),
  },
  attachments: {
    upload: (roomId: number, file: File): Promise<MessageAttachment> => {
      const form = new FormData();
      form.append("file", file, file.name);
      return requestJson<MessageAttachment>(
        `/api/rooms/${roomId}/attachments`,
        { method: "POST", body: form },
      );
    },
    url: (id: number): string => apiUrl(`/api/attachments/${id}`),
    remove: (id: number): Promise<void> =>
      requestJson<void>(`/api/attachments/${id}`, { method: "DELETE" }),
  },
  runs: {
    /** 房间最近运行，按创建时间倒序；活动只在进程内保留。 */
    list: (roomId: number, limit = 20): Promise<AgentRun[]> =>
      requestJson<AgentRun[]>(`/api/rooms/${roomId}/runs?limit=${limit}`),
    /** 单次运行详情：指令、结果、错误与本次进程内产生的活动。 */
    detail: (roomId: number, runId: number): Promise<RunDetailResponse> =>
      requestJson<RunDetailResponse>(`/api/rooms/${roomId}/runs/${runId}`),
  },
  discussions: {
    /** 房间最新讨论；没有讨论时返回 null。 */
    latest: (roomId: number): Promise<Discussion | null> =>
      requestJson<Discussion | null>(
        `/api/rooms/${roomId}/discussions/latest`,
      ),
    /** 用户手动停止当前讨论；已结束时幂等返回最新快照。 */
    stop: (roomId: number, discussionId: number): Promise<Discussion> =>
      requestJson<Discussion>(
        `/api/rooms/${roomId}/discussions/${discussionId}/stop`,
        { method: "POST" },
      ),
    /** 续跑失败讨论：沿用原讨论从断点继续，不新建讨论。 */
    resume: (roomId: number, discussionId: number): Promise<Discussion> =>
      requestJson<Discussion>(
        `/api/rooms/${roomId}/discussions/${discussionId}/resume`,
        { method: "POST" },
      ),
  },
  collaborations: {
    /** 房间最新委派批次；没有批次时返回 null。 */
    latest: (roomId: number): Promise<CollaborationBatch | null> =>
      requestJson<CollaborationBatch | null>(
        `/api/rooms/${roomId}/collaborations/latest`,
      ),
    /** 停止当前批次；已结束时幂等返回最新快照。 */
    stop: (roomId: number, batchId: number): Promise<CollaborationBatch> =>
      requestJson<CollaborationBatch>(
        `/api/rooms/${roomId}/collaborations/${batchId}/stop`,
        { method: "POST" },
      ),
  },
  tasks: {
    list: (roomId: number): Promise<Task[]> =>
      requestJson<Task[]>(`/api/rooms/${roomId}/tasks`),
    create: (roomId: number, input: TaskCreateInput): Promise<Task> =>
      requestJson<Task>(`/api/rooms/${roomId}/tasks`, {
        method: "POST",
        body: JSON.stringify(input),
      }),
    /** 更新可编辑字段；非法定时/指派返回 400，任务不存在 404。 */
    update: (taskId: number, input: TaskUpdateInput): Promise<Task> =>
      requestJson<Task>(`/api/tasks/${taskId}`, {
        method: "PATCH",
        body: JSON.stringify(input),
      }),
    /** 受控流转；非法迁移由服务端返回 409 INVALID_TRANSITION。 */
    updateStatus: (taskId: number, status: TaskStatus): Promise<Task> =>
      requestJson<Task>(`/api/tasks/${taskId}/status`, {
        method: "PATCH",
        body: JSON.stringify({ status }),
      }),
    /** 启动主持人编排：202 表示已异步启动，进展经 WS 推送。 */
    activate: (taskId: number): Promise<Task> =>
      requestJson<Task>(`/api/tasks/${taskId}/activate`, {
        method: "POST",
      }),
    /** 完整历史时间线：任务事件 + 任务关联消息 + 运行记录，按时间升序。 */
    timeline: (taskId: number): Promise<TaskTimelineResponse> =>
      requestJson<TaskTimelineResponse>(`/api/tasks/${taskId}/timeline`),
  },
  orchestration: {
    /** 读取房间级暂停状态；页面加载时用它水合，避免刷新后显示错误。 */
    getState: (roomId: number): Promise<OrchestrationPauseState> =>
      requestJson<OrchestrationPauseState>(
        `/api/rooms/${roomId}/orchestration`,
      ),
    pause: (roomId: number): Promise<OrchestrationPauseState> =>
      requestJson<OrchestrationPauseState>(
        `/api/rooms/${roomId}/orchestration/pause`,
        { method: "POST" },
      ),
    resume: (roomId: number): Promise<OrchestrationPauseState> =>
      requestJson<OrchestrationPauseState>(
        `/api/rooms/${roomId}/orchestration/resume`,
        { method: "POST" },
      ),
  },
};

/** 页面展示用：统一把未知异常转成可读文案。 */
export function errorMessage(error: unknown): string {
  if (error instanceof ApiError) {
    return error.message;
  }
  if (error instanceof Error) {
    return error.message;
  }
  return "未知错误";
}
