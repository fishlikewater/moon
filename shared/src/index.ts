/**
 * Moon 前后端共享契约类型的唯一来源。
 *
 * 领域类型（Agent / Room / Message / Task / AgentRun 等）集中定义于此，
 * 前后端禁止各自手写重复接口。
 *
 * 时间字段约定：所有 `*At` 时间字段统一为 epoch 毫秒数字（number），
 * 与 SQLite 的 INTEGER 列一一对应。
 */
export const SHARED_PACKAGE_NAME = "@moon/shared";

/** harness 类型枚举；`zcode` / `codex` / `fake` 已实现，`kimi-code` 为占位，`acp` 走自定义注册表。 */
export type HarnessKind = "codex" | "zcode" | "kimi-code" | "fake" | "acp";

/**
 * 智能体模型配置，序列化后存入 `agents.model_config_json`。
 * 具体字段语义由各 harness 适配器解释，未识别字段原样保留。
 */
export interface ModelConfig {
  model?: string;
  /** 模型支持的思考等级；留空表示使用模型默认。 */
  reasoningLevel?: string;
  /** ACP 会话模式（agent 宣告的 mode id）；运行时经 session/set_mode 下发，优先于房间权限映射。 */
  sessionMode?: string;
  [key: string]: unknown;
}

/** harness 模型下拉中的一个可选项。 */
export interface HarnessModelOption {
  /** 前端 option 的稳定值；服务端按 providerId + modelId 无歧义生成。 */
  key: string;
  providerId: string;
  providerName: string;
  modelId: string;
  /** 该模型可选的思考等级；空数组表示未声明。 */
  reasoningLevels: string[];
  /** 模型默认思考等级；null 表示未声明。 */
  defaultReasoningLevel: string | null;
}

/** `GET /api/harnesses/:harness/models` 响应。 */
export interface HarnessModelCatalog {
  harness: HarnessKind;
  /** 是否成功读取到可选项；false 时仍可留空使用 harness 默认模型。 */
  available: boolean;
  models: HarnessModelOption[];
  /** 不可用原因；正常为 null。 */
  message: string | null;
}

/** 创建助手对话消息；assistant 内容不包含结构化草稿块。 */
export interface AgentBuilderMessage {
  role: "user" | "assistant";
  content: string;
}

/** 创建助手整理出的智能体草稿；空字符串表示该字段尚未确定。 */
export interface AgentDraft {
  name: string;
  persona: string;
  harness: HarnessKind;
  modelConfig: ModelConfig | null;
}

/** `POST /api/agent-builder/chat` 请求。 */
export interface AgentBuilderChatRequest {
  messages: AgentBuilderMessage[];
  /** 助手自身使用的模型配置；省略或 null 表示使用 harness 默认模型。 */
  assistantModelConfig?: ModelConfig | null;
}

/** 创建助手单轮运行的元数据；不落库，只随响应展示。 */
export interface AgentBuilderRunInfo {
  status: "completed" | "failed";
  /** 实际请求的模型；使用 harness 默认模型时为 null。 */
  model: string | null;
  durationMs: number | null;
  tokenCount: number | null;
  toolCallCount: number | null;
  contextSize: number | null;
}

/** `POST /api/agent-builder/chat` 响应。 */
export interface AgentBuilderChatResponse {
  /** 已移除 `moon-agent-draft` 代码块的展示文本。 */
  reply: string;
  /** 本轮没有可解析草稿时为 null；前端应保留上一版草稿。 */
  draft: AgentDraft | null;
  /** 需要用户注意的校验提示，例如重名或模型不在目录中。 */
  notes: string[];
  /** 本轮运行的模型、耗时与用量。 */
  run: AgentBuilderRunInfo;
}

/** 内置智能体角色；当前只有房间主持人。 */
export type BuiltinAgentRole = "moderator";

/**
 * 自定义 ACP agent 注册表条目：一个可执行的本地 ACP 程序。
 *
 * `command` 是可执行文件路径或 PATH 可解析名称；`args` 原样传给进程。
 */
export interface CustomHarness {
  id: number;
  name: string;
  command: string;
  args: string[];
  /** 环境变量覆盖；与父进程环境合并后传给子进程。 */
  env: Record<string, string>;
  /** epoch 毫秒 */
  createdAt: number;
  /** 引用该定义的智能体数量；服务端实时统计。 */
  agentCount: number;
  /** 最近一次探测到的 ACP 能力；未探测为 null。 */
  capabilities: AcpCapabilities | null;
  /** 最近一次探测时间；未探测为 null。 */
  probedAt: number | null;
}

/** ACP agent 自报的模式与模型能力快照。 */
export interface AcpCapabilities {
  models: AcpModelOption[];
  currentModelId: string | null;
  modes: AcpMode[];
  currentModeId: string | null;
  configOptions: AcpConfigOption[];
}

export interface AcpMode {
  id: string;
  name: string;
}

export interface AcpModelOption {
  value: string;
  name: string;
}

export interface AcpConfigOption {
  id: string;
  name: string;
  category: string | null;
  /** ACP 的 `type` 字段；select 才带 options。 */
  optionType: string | null;
  currentValue: string | null;
  options: AcpConfigChoice[];
}

export interface AcpConfigChoice {
  value: string;
  name: string;
}

/** `POST /api/custom-harnesses/:id/probe` 响应。 */
export interface AcpProbeResponse {
  models: AcpModelOption[];
  modes: AcpMode[];
  configOptions: AcpConfigOption[];
  currentModelId: string | null;
  currentModeId: string | null;
  probedAt: number;
}

export interface CustomHarnessCreateInput {
  name: string;
  command: string;
  args?: string[];
  env?: Record<string, string>;
}

export type CustomHarnessUpdateInput = Partial<CustomHarnessCreateInput>;

export interface Agent {
  id: number;
  name: string;
  persona: string;
  harness: HarnessKind;
  modelConfig: ModelConfig | null;
  /** 非 null 表示系统内置智能体；内置主持人不可删除或改身份字段。 */
  builtinRole: BuiltinAgentRole | null;
  /** harness 为 acp 时指向自定义 ACP agent 注册表条目。 */
  customHarnessId: number | null;
  /** 智能体独立工作区；null 表示使用房间主工作区。 */
  workspaceDir: string | null;
  /** epoch 毫秒 */
  createdAt: number;
}

/** 房间默认回复模式：仅响应 @ 提及，或由内置主持人接管无 @ 消息。 */
export type RoomReplyMode = "mentions" | "moderator";

/**
 * 房间级 harness 权限模式。
 *
 * - `plan`：计划模式，只读规划；
 * - `auto`：自主模式，可在工作区读写与执行，危险操作由 harness 拦截；
 * - `full`：完全访问，无审批、无沙箱。
 *
 * 具体 CLI 参数由各 harness 适配器映射；Moon 层不实现应用级沙箱。
 */
export type RoomPermissionMode = "plan" | "auto" | "full";

export const ROOM_PERMISSION_MODES = ["plan", "auto", "full"] as const;

/** 新房间默认自主模式；已有房间由 v10 迁移保持为 `full`。 */
export const DEFAULT_ROOM_PERMISSION_MODE: RoomPermissionMode = "auto";

/**
 * 服务端持久化的应用级设置。
 *
 * 主题、侧栏折叠等纯浏览器偏好仍在 localStorage，不在此契约内。
 */
export interface AppSettings {
  defaultRoomPermissionMode: RoomPermissionMode;
  defaultRoomDiscussionRounds: number;
  contextCompressionEnabled: boolean;
  contextBudgetChars: number;
  attachmentMaxFileBytes: number;
  attachmentMaxFilesPerMessage: number;
  /** 运行总时长上限（秒）；0 表示不限制。 */
  runTimeoutSecs: number;
}

/** `GET /api/settings` 响应。 */
export interface AppSettingsResponse {
  settings: AppSettings;
}

export interface Room {
  id: number;
  name: string;
  /** 主工作目录，也是 harness 的运行 cwd。 */
  workspaceDir: string;
  /** 房间绑定的全部目录；第一个与 workspaceDir 相同。 */
  workspaceDirs: string[];
  /** 主持人必须是房间成员；内置主持人方案下始终指向内置主持人 */
  moderatorAgentId: number | null;
  /** 房间成员 id，按 id 升序（对应 `room_members`） */
  memberIds: number[];
  /** 默认回复模式；API 缺省为 mentions，新房间由前端选择 moderator。 */
  replyMode: RoomReplyMode;
  /** 单次讨论允许的最大轮数上限，1–50；主持人发起时可在该范围内选择。 */
  discussionMaxRounds: number;
  /** 房间级 harness 权限模式；所有成员、讨论与委派运行共用。 */
  permissionMode: RoomPermissionMode;
  /** epoch 毫秒 */
  createdAt: number;
  /** 最近一条消息时间（epoch 毫秒）；无消息为 null。仅 `GET /api/rooms` 列表响应填充，其余房间接口返回 null。 */
  lastActivityAt: number | null;
}

/** 房间讨论轮数上限的合法范围与默认值。 */
export const MIN_ROOM_DISCUSSION_ROUNDS = 1;
export const MAX_ROOM_DISCUSSION_ROUNDS = 50;
export const DEFAULT_ROOM_DISCUSSION_ROUNDS = 5;

/** 房间文件清单中的单个文件；path 相对所属根目录。 */
export interface RoomFileEntry {
  path: string;
  size: number;
  /** epoch 毫秒；无法读取时为 0。 */
  modifiedAt: number;
}

/** 一个可浏览的工作区根：房间工作区或成员智能体工作区。 */
export interface RoomFileRoot {
  /** 下载接口使用的根 id：`room-<index>` 或 `agent-<id>`。 */
  id: string;
  label: string;
  kind: "room" | "agent";
  agentId: number | null;
  path: string;
  files: RoomFileEntry[];
  /** 达到单根文件数上限时为 true，清单不是全量。 */
  truncated: boolean;
}

export interface RoomFiles {
  roomId: number;
  roots: RoomFileRoot[];
}

export type MessageSenderType = "user" | "agent" | "system";
export type MessageStatus = "final" | "streaming" | "failed";

/**
 * 消息用途。
 *
 * - `reply`：正式回复或系统/用户消息；参与上下文、讨论轮次与委派产出。
 * - `progress`：智能体执行期间的阶段总结；只用于展示，不进入其他智能体的上下文。
 */
export type MessageKind = "reply" | "progress";

/** 讨论生命周期状态；同一房间只允许一个 running 讨论。 */
export type DiscussionStatus = "running" | "completed" | "stopped" | "failed";

/** 讨论结束原因；running 时为 null。 */
export type DiscussionStopReason =
  | "all_voted_stop"
  | "moderator_decision"
  | "max_rounds"
  | "user_stop"
  | "participant_failed"
  | "moderator_failed"
  | "interrupted"
  | "internal_error";

/** 参与者对当前议题的立场。 */
export type DiscussionStance = "propose" | "support" | "oppose" | "question";

/** 参与者对本轮结束后是否继续讨论的投票。 */
export type DiscussionStopVote = "continue" | "stop";

/** 房间内一次有界讨论的持久化快照。 */
export interface Discussion {
  id: number;
  roomId: number;
  topic: string;
  status: DiscussionStatus;
  currentRound: number;
  maxRounds: number;
  /** 参与者按发言顺序排列，不含主持人。 */
  participantIds: number[];
  /** 发起讨论的主持人 id。 */
  createdByAgentId: number | null;
  stopReason: DiscussionStopReason | null;
  /** 主持人最终总结；未结束时为 null。 */
  summary: string | null;
  /** epoch 毫秒 */
  createdAt: number;
  /** epoch 毫秒 */
  updatedAt: number;
}

/** 消息所属讨论的轮次与结构化发言元数据。 */
export interface MessageDiscussion {
  discussionId: number;
  round: number;
  stance: DiscussionStance | null;
  /** 被回复消息 id；主持人裁决与系统消息为 null。 */
  replyToMessageId: number | null;
  stopVote: DiscussionStopVote | null;
  /** 参与者自己的停止理由；仅 stopVote 为 stop 时通常有值。 */
  stopReason: string | null;
}

export interface Message {
  id: number;
  roomId: number;
  /** 房间内严格递增的序号，跨房间独立计数 */
  seq: number;
  senderType: MessageSenderType;
  /** 仅 `senderType === "agent"` 时有值 */
  senderAgentId: number | null;
  status: MessageStatus;
  /** 消息用途；默认 `reply`，兼容旧数据与所有非运行消息。 */
  kind: MessageKind;
  /** 关联运行；仅智能体运行消息有值，历史消息为 null。 */
  runId: number | null;
  content: string;
  /** 关联任务；普通聊天为 null。任务时间线按此字段聚合。 */
  taskId: number | null;
  /** 普通消息的回复目标；讨论回复使用 discussion.replyToMessageId。 */
  replyToMessageId?: number | null;
  /** 讨论元数据；非讨论消息不携带该字段。 */
  discussion?: MessageDiscussion | null;
  /** 委派批次元数据；非委派消息不携带该字段。 */
  collaboration?: MessageCollaboration | null;
  /** 用户消息附件；旧数据可能缺省。 */
  attachments?: MessageAttachment[];
  /** 表情反应；旧数据可能缺省。 */
  reactions?: MessageReaction[];
  /** epoch 毫秒 */
  createdAt: number;
}

/** 固定调色板内的消息表情反应。 */
export interface MessageReaction {
  emoji: string;
  /** 同一 emoji 的反应数；单用户产品通常为 1。 */
  count: number;
  /** 当前本地用户是否已添加该反应。 */
  reactedByMe: boolean;
}

/** 消息附件；磁盘路径只在服务端使用，不返回前端。 */
export interface MessageAttachment {
  id: number;
  name: string;
  mimeType: string;
  size: number;
  /** epoch 毫秒 */
  createdAt: number;
}

/** 委派批次生命周期；同一时刻同一房间只应有一个 running 批次。 */
export type CollaborationStatus =
  | "running"
  | "completed"
  | "stopped"
  | "failed";

/** 批次结束原因；running 时为 null。 */
export type CollaborationStopReason =
  | "moderator_stop"
  | "max_rounds"
  | "user_stop"
  | "interrupted"
  | "invalid_next_assignments"
  | "moderator_failed"
  | "internal_error";

/** 单条指派的生命周期。 */
export type CollaborationAssignmentStatus =
  | "queued"
  | "running"
  | "succeeded"
  | "failed"
  | "skipped";

/** 消息在批次中的角色；系统提示也归入批次便于轮次分组。 */
export type CollaborationMessageRole = "assignment" | "summary" | "system";

/** 批次内一条成员指派。 */
export interface CollaborationAssignment {
  id: number;
  batchId: number;
  round: number;
  agentId: number;
  instruction: string;
  status: CollaborationAssignmentStatus;
  runId: number | null;
  messageId: number | null;
  error: string | null;
  /** epoch 毫秒 */
  createdAt: number;
  /** epoch 毫秒；未结束时为 null */
  finishedAt: number | null;
}

/** 主持人委派的一个有界批次。 */
export interface CollaborationBatch {
  id: number;
  roomId: number;
  moderatorAgentId: number;
  /** 触发批次的主持人回复之前那条用户消息。 */
  userMessageId: number;
  status: CollaborationStatus;
  currentRound: number;
  maxRounds: number;
  stopReason: CollaborationStopReason | null;
  /** 用户请求停止；当前轮结束后不再派发下一轮。 */
  stopRequested: boolean;
  /** 主持人最终汇总；未结束时为 null。 */
  summary: string | null;
  /** 正在生成或已生成当前轮汇总的主持人运行 id。 */
  reviewRunId: number | null;
  /** 按轮次与创建顺序排列。 */
  assignments: CollaborationAssignment[];
  /** epoch 毫秒 */
  createdAt: number;
  /** epoch 毫秒 */
  updatedAt: number;
}

/** 消息所属委派批次的轮次与角色。 */
export interface MessageCollaboration {
  batchId: number;
  round: number;
  role: CollaborationMessageRole;
}

/** 委派默认轮次上限与单轮最大指派数。 */
export const DEFAULT_COLLABORATION_MAX_ROUNDS = 3;
export const MAX_COLLABORATION_ASSIGNMENTS = 5;

/**
 * 房间实时事件（WebSocket `/ws?roomId=<id>` 推送）。
 *
 * 前后端唯一契约来源；服务端只向订阅了该房间的连接广播。
 * 所有事件都带 `roomId`，客户端可据此丢弃跨房间的过期事件。
 */
export type RoomEvent =
  | {
      type: "run_started";
      roomId: number;
      runId: number;
      agentId: number;
      messageId: number;
      /** 已落库的 streaming 占位消息，客户端可直接插入列表 */
      message: Message;
    }
  | {
      /** 同一运行进入新阶段：旧阶段已落定为 progress，这条是新的 streaming 候选 */
      type: "message_started";
      roomId: number;
      runId: number;
      /** 已落库的 streaming 占位消息，客户端可直接插入列表 */
      message: Message;
    }
  | {
      type: "message_delta";
      roomId: number;
      messageId: number;
      /** 增量文本；客户端按 messageId 累积 */
      text: string;
    }
  | {
      type: "message_final";
      roomId: number;
      /** 落定后的完整消息（用户消息、智能体回复、system 消息共用） */
      message: Message;
    }
  | {
      /** 某条消息的表情反应已切换；客户端只替换该消息的 reactions */
      type: "message_reactions_changed";
      roomId: number;
      messageId: number;
      reactions: MessageReaction[];
    }
  | {
      /** 任务状态已在事务内提交；客户端据此实时更新看板 */
      type: "task_changed";
      roomId: number;
      taskId: number;
      status: TaskStatus;
    }
  | {
      /** 讨论创建、轮次推进或结束时推送最新快照 */
      type: "discussion_changed";
      roomId: number;
      discussion: Discussion;
    }
  | {
      /** 委派批次创建、轮次推进、停止或结束时推送最新快照 */
      type: "collaboration_changed";
      roomId: number;
      batch: CollaborationBatch;
    }
  | {
      /** 运行期间产生一条可展示活动；客户端按 runId + seq 追加 */
      type: "run_activity";
      roomId: number;
      activity: RunActivity;
    }
  | {
      /** 纯聊天历史已清理；客户端应重新拉取消息列表 */
      type: "history_cleared";
      roomId: number;
    };

export interface MessageCleanupResult {
  /** 实际删除的消息条数 */
  deleted: number;
  /** 实际删除的归档文件数 */
  artifactsDeleted: number;
  /** 实际删除附件磁盘文件数 */
  attachmentsDeleted: number;
}

export type RoomEventType = RoomEvent["type"];

export type TaskStatus =
  | "backlog"
  | "in_progress"
  | "review"
  | "done"
  | "blocked"
  | "cancelled";

/** 任务优先级；历史任务缺省 normal。 */
export type TaskPriority = "low" | "normal" | "high" | "urgent";

/**
 * 任务锚点：主持人创建任务时写入的目标、验收标准、验证方式与范围。
 * 借鉴 cowork-flow 的 decision-anchor，但不依赖其运行时。
 */
export interface TaskPlan {
  goal: string;
  acceptanceCriteria: string[];
  verification: string[];
  scope: string[];
}

export interface Task {
  id: number;
  roomId: number;
  title: string;
  description: string;
  status: TaskStatus;
  /** 优先级；历史任务缺省 normal。 */
  priority: TaskPriority;
  /** 指派的房间成员；未指派为 null。 */
  assigneeAgentId: number | null;
  /** 截止时间（epoch 毫秒）；未设置为 null。 */
  dueAt: number | null;
  /** 定时自动启动时间（epoch 毫秒）；仅 backlog 可设置，启动/取消后清除。 */
  scheduledAt: number | null;
  /** 结构化任务计划；手工创建的任务可以为 null。 */
  plan: TaskPlan | null;
  /** 主持人创建时记录发起者；手工任务为 null。 */
  createdByAgentId: number | null;
  /** epoch 毫秒 */
  createdAt: number;
  /** epoch 毫秒 */
  updatedAt: number;
}

export interface TaskEvent {
  id: number;
  taskId: number;
  /** 事件类型，如 `created`、`status_changed` */
  type: string;
  /** 解析后的 `payload_json`；无载荷时为 null */
  payload: Record<string, unknown> | null;
  /** epoch 毫秒 */
  createdAt: number;
}

export type RunStatus = "running" | "succeeded" | "failed" | "interrupted";

export interface AgentRun {
  id: number;
  roomId: number;
  agentId: number;
  /** 编排场景下关联任务；普通聊天为 null */
  taskId: number | null;
  status: RunStatus;
  prompt: string;
  result: string | null;
  error: string | null;
  /** 本次运行使用的 harness 会话 ID；会话复用或首次建立会话时有值。 */
  harnessSessionId: string | null;
  /** harness 上报的 token 数；未知或旧运行缺省。 */
  tokenCount?: number | null;
  /** harness 上报的上下文大小；未知或旧运行缺省。 */
  contextSize?: number | null;
  /** epoch 毫秒 */
  createdAt: number;
  /** 终态时间；运行中为 null，epoch 毫秒 */
  finishedAt: number | null;
}

/** 发送前的上下文估算；与真实运行的房间历史压缩共用同一逻辑。 */
export interface ContextPreview {
  enabled: boolean;
  budgetChars: number;
  /** 压缩前，被考虑消息的正文与附件清单字符数。 */
  rawChars: number;
  /** 实际进入提示词的历史转录字符数；复用会话时为增量指令字符数。 */
  effectiveChars: number;
  /** 固定 ceil(chars / 4)，只用于展示。 */
  estimatedTokens: number;
  includedMessages: number;
  compressedMessages: number;
  omittedMessages: number;
  sessionReused: boolean;
}

/** 队列任务类型；讨论/委派/任务运行不可从队列取消。 */
export type QueueJobKind =
  | "chat"
  | "moderator"
  | "discussion"
  | "collaboration"
  | "task";

export type QueueJobState = "queued" | "running";

/** 房间队列中的单个任务；运行中 position 为 0，排队从 1 开始。 */
export interface QueueJob {
  id: number;
  roomId: number;
  agentId: number | null;
  label: string;
  kind: QueueJobKind;
  state: QueueJobState;
  position: number;
  enqueuedAt: number;
  startedAt: number | null;
}

export interface QueueSnapshot {
  roomId: number;
  /** 暂停只阻止后续任务启动；运行中的任务不中断。 */
  paused: boolean;
  jobs: QueueJob[];
}

/** 时间线条目类型：任务审计事件 / 消息 / harness 运行。 */
export type TimelineEntryKind = "task_event" | "message" | "run";

/**
 * 任务时间线条目。
 *
 * 统一按 `createdAt` 升序；同一毫秒内按 `task_event → run → message`
 * 排序，再按各自 `id` 升序，保证前后端看到同一稳定次序。
 */
export type TimelineEntry =
  | {
      kind: "task_event";
      /** 来源表 `task_events.id` */
      id: number;
      taskId: number;
      createdAt: number;
      event: TaskEvent;
    }
  | {
      kind: "message";
      /** 来源表 `messages.id` */
      id: number;
      taskId: number;
      createdAt: number;
      message: Message;
    }
  | {
      kind: "run";
      /** 来源表 `agent_runs.id` */
      id: number;
      taskId: number;
      createdAt: number;
      run: AgentRun;
    };

/** `GET /api/tasks/:id/timeline` 响应。 */
export interface TaskTimelineResponse {
  taskId: number;
  entries: TimelineEntry[];
}

/** 运行活动类型：推理、工具调用、用量与阶段状态。 */
export type RunActivityKind = "reasoning" | "tool" | "usage" | "status";

/**
 * 一次 harness 运行期间的一条可展示活动。
 *
 * 只用于实时可观测与最近运行回看；不落库，进程重启后即失效。
 * `seq` 在单个运行内从 1 递增，前端据此去重与排序。
 */
export interface RunActivity {
  runId: number;
  seq: number;
  kind: RunActivityKind;
  /** 简短标题，例如工具名或阶段名。 */
  title: string;
  /** 已由服务端截断的明细文本；没有明细时为 null。 */
  detail: string | null;
  /** epoch 毫秒 */
  at: number;
}

/** `GET /api/rooms/:id/runs/:runId` 响应。 */
export interface RunDetailResponse {
  run: AgentRun;
  /** 按 `seq` 升序；无活动或进程已重启时为空数组。 */
  activities: RunActivity[];
  /** 该运行产生的阶段总结与最终回复，按 seq 升序；旧运行可能为空。 */
  messages: Message[];
}

/** 房间内智能体会话状态；归档后不再被运行续接。 */
export type AgentSessionStatus = "active" | "archived";

/**
 * 房间内一个智能体的 harness 会话。
 *
 * 隔离键固定为 `(roomId, agentId)`：同一智能体在不同房间各自独立，
 * 跨房间不共享 sessionId 或本地存储。`storage_dir` 属于服务端内部路径，
 * 不进入对外响应。
 */
export interface AgentSession {
  id: number;
  roomId: number;
  agentId: number;
  harness: HarnessKind;
  /** harness 侧会话 ID（zcode `sess_*` / codex UUID）；首次运行解析前为 null。 */
  harnessSessionId: string | null;
  status: AgentSessionStatus;
  /** epoch 毫秒 */
  createdAt: number;
  /** 最近一次使用时间；从未使用时为 null，epoch 毫秒 */
  lastUsedAt: number | null;
  /** 最近一次使用该会话的运行 id；从未使用时为 null。 */
  lastRunId: number | null;
}

/** 房间成员的会话复用设置。 */
export interface RoomMemberSessionSettings {
  agentId: number;
  /** true 表示该成员在同一房间内的连续运行续接同一 harness 会话。 */
  reuseSession: boolean;
}

/** 全局会话总览：会话字段 + 房间与智能体名称；`GET /api/sessions` 元素。 */
export interface SessionOverview extends AgentSession {
  roomName: string;
  agentName: string;
}

/** `GET /api/rooms/:id/sessions` 响应。 */
export interface RoomSessionsResponse {
  /** 房间全部成员及其复用开关。 */
  members: RoomMemberSessionSettings[];
  /** 房间内全部会话（含归档），按创建顺序倒序。 */
  sessions: AgentSession[];
}

/** 适配器运行使用的会话上下文；不复用会话的运行不传。 */
export interface RunSessionContext {
  /** 已知 harness 会话 ID；null 表示本次运行需要建立新会话。 */
  harnessSessionId: string | null;
  /** 会话隔离存储目录（绝对路径，由服务层解析）。 */
  storageDir: string;
}
