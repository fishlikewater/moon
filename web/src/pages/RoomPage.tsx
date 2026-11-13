import {
  Fragment,
  lazy,
  memo,
  Suspense,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ChangeEvent,
  type ClipboardEvent,
  type DragEvent,
  type FormEvent,
  type KeyboardEvent,
} from "react";
import { Link, useNavigate, useParams } from "react-router-dom";
import type {
  Agent,
  CollaborationBatch,
  ContextPreview,
  Discussion,
  Message,
  MessageAttachment,
  MessageStatus,
  QueueSnapshot,
  Room,
  RoomEvent,
  RoomFiles,
} from "@moon/shared";
import { api, errorMessage } from "../api/client";
import { AgentAvatar } from "../components/AgentAvatar";
import { composerKeyAction } from "../components/composerKeys";
import { fileDownloadUrl } from "../components/filesView";
import {
  nextRoomSideTab,
  persistRoomSideTab,
  readRoomSideTab,
  ROOM_SIDE_TABS,
  type RoomSideTab,
} from "../components/roomSidebarTabs";
import {
  formatContextMetric,
  formatContextSummary,
} from "../components/contextView";
import {
  canCancelQueueJob,
  formatWait,
  queueHeadline,
  queueKindLabel,
  queueStateLabel,
  sortQueueJobs,
} from "../components/queueView";
import {
  DEFAULT_ATTACHMENT_LIMITS,
  filterPendingFiles,
  formatFileSize,
  isImageAttachment,
  type AttachmentLimits,
} from "../components/attachmentView";
import {
  applyReactionToggle,
  isInteractiveMessage,
  REACTION_EMOJIS,
  splitMentions,
  summarizeReply,
} from "../components/messageInteractions";
import SessionPanel from "../components/SessionPanel";
import { buildRoomChrome, useRoomChrome } from "../components/layout/roomChrome";
import {
  extractHeadings,
  filterMessages,
  formatElapsed,
  groupMessages,
  moderatorMessageClass,
  shouldCollapse,
  type MessageFilter,
} from "../components/messageView";
import {
  assignmentStatusLabel,
  assignmentsForRound,
  collaborationRoleLabel,
  collaborationStatusLabel,
  collaborationStopReasonLabel,
  countAssignments,
  formatCollaborationRound,
  shouldShowRoundDivider,
} from "../components/collaborationView";
import {
  discussionMessageKind,
  discussionStanceLabel,
  discussionStatusLabel,
  discussionStopReasonLabel,
  discussionStopVoteLabel,
  formatDiscussionConclusion,
  formatDiscussionRound,
  formatParticipantNames,
  isResumableDiscussion,
  plainDiscussionSummary,
  summarizeQuote,
} from "../components/discussionView";
import { formatModeratorTranscript } from "../components/moderatorProtocol";
import { useRoomEvents } from "../ws/useRoomEvents";
import {
  cleanupPreview,
  parseKeepRecent,
} from "./roomCleanup";
import { formatCleanupResult } from "./roomCleanupResult";

// Markdown 依赖体积较大；按需加载，避免拖慢房间页首屏与主 chunk。
const MarkdownContent = lazy(() => import("../components/MarkdownContent"));

/** 右侧操作栏图标：统一 24 网格线性描边。 */
function HistoryIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.7"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      <path d="M3 12a9 9 0 1 0 2.6-6.3L3 8" />
      <path d="M3 3v5h5" />
      <path d="M12 7v5l3 2" />
    </svg>
  );
}

function SessionsIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.7"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      <path d="M21 11.5a8.5 8.5 0 0 1-8.5 8.5H4l2.2-3.3A8.5 8.5 0 1 1 21 11.5Z" />
      <path d="M8 11h8" />
      <path d="M8 14.5h5" />
    </svg>
  );
}

function TrashIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.7"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      <path d="M4 7h16" />
      <path d="M10 11v6" />
      <path d="M14 11v6" />
      <path d="M6 7l1 13h10l1-13" />
      <path d="M9 7V4h6v3" />
    </svg>
  );
}

function PanelIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.7"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      <rect x="3" y="4" width="18" height="16" rx="2" />
      <path d="M15 4v16" />
    </svg>
  );
}

function ChevronDownIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      <path d="m6 9 6 6 6-6" />
    </svg>
  );
}

function PaperclipIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.7"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      <path d="M21 11.5 12.5 20a5 5 0 0 1-7.1-7.1l8.5-8.5a3.3 3.3 0 0 1 4.7 4.7l-8.5 8.5a1.7 1.7 0 0 1-2.4-2.4l7.8-7.8" />
    </svg>
  );
}

function ArrowUpIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2.2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      <path d="M12 19V5" />
      <path d="m5.5 11.5 6.5-6.5 6.5 6.5" />
    </svg>
  );
}

interface ActiveRun {
  runId: number;
  agentId: number;
  messageId: number;
  startedAt: number;
}

interface MemberView {
  id: number;
  name: string;
  builtinRole: Agent["builtinRole"];
}

interface CompletionOption {
  key: string;
  label: string;
  description: string;
  insert: string;
}

interface ModeratorCommand {
  name: string;
  description: string;
  template: string;
}

/** `/` 命令只是一段可直接编辑的提示词模板，不读取工作区或执行命令。 */
const MODERATOR_COMMANDS: readonly ModeratorCommand[] = [
  {
    name: "计划",
    description: "拆解目标、验收标准与验证方式",
    template:
      "请为当前目标制定执行计划：\n- 目标：\n- 验收标准：\n- 验证方式：\n- 分工建议：\n",
  },
  {
    name: "委派",
    description: "安排成员并写清预期产出",
    template:
      "请安排合适的成员执行，并写清每位成员的预期产出、验收标准与复核方式。\n",
  },
  {
    name: "总结",
    description: "汇总进展、风险与下一步",
    template: "请总结当前进展、已完成项、未完成项、风险与下一步。\n",
  },
  {
    name: "评审",
    description: "安排复核并给出验收证据",
    template:
      "请安排成员复核当前产出，对照验收标准给出通过/不通过结论与可核对证据。\n",
  },
];

/**
 * 找出光标前正在输入的 `@成员` 或 `/命令` token。
 *
 * 只识别以 @ 或 / 开头的连续非空白 token，避免把邮箱、路径等误判为补全。
 */
export function findCompletionTrigger(
  text: string,
  cursor: number,
): { kind: "mention" | "command"; start: number; end: number; query: string } | null {
  const clamped = Math.max(0, Math.min(cursor, text.length));
  const before = text.slice(0, clamped);
  let start = before.length;
  while (start > 0 && !/\s/.test(before[start - 1] ?? "")) {
    start -= 1;
  }
  const token = before.slice(start);

  if (token.startsWith("@")) {
    return { kind: "mention", start, end: clamped, query: token.slice(1) };
  }
  if (token.startsWith("/")) {
    return { kind: "command", start, end: clamped, query: token.slice(1) };
  }
  return null;
}

/** 消息状态优先级：终态（final/failed）高于流式中的占位消息。 */
const MESSAGE_STATUS_PRIORITY: Record<MessageStatus, number> = {
  streaming: 0,
  final: 1,
  failed: 1,
};

/**
 * 同一 id 的两份快照取“更新”的一份：终态优先于 streaming，
 * 同状态取内容更长的一份（流式增量只会增长）。
 */
function pickNewerMessage(left: Message, right: Message): Message {
  const leftPriority = MESSAGE_STATUS_PRIORITY[left.status];
  const rightPriority = MESSAGE_STATUS_PRIORITY[right.status];
  if (leftPriority !== rightPriority) {
    return rightPriority > leftPriority ? right : left;
  }
  return right.content.length >= left.content.length ? right : left;
}

/**
 * 合并两份消息快照：按 id 去重、按 seq 升序。
 *
 * 历史加载完成时用它与已到达的实时消息合并，而不是直接覆盖状态；
 * 同一 id 的优先级规则见 `pickNewerMessage`。
 */
export function mergeMessages(
  current: readonly Message[],
  incoming: readonly Message[],
): Message[] {
  const byId = new Map<number, Message>();
  for (const message of current) {
    byId.set(message.id, message);
  }
  for (const message of incoming) {
    const existing = byId.get(message.id);
    byId.set(
      message.id,
      existing === undefined ? message : pickNewerMessage(existing, message),
    );
  }
  return [...byId.values()].sort((a, b) => a.seq - b.seq);
}

/**
 * 事件是否属于当前房间。契约要求所有事件都带 `roomId`，
 * 切换房间后旧连接可能仍投递事件，必须在此丢弃。
 */
export function isEventForRoom(
  event: RoomEvent,
  roomId: number | null,
): boolean {
  return roomId !== null && event.roomId === roomId;
}

/** 消息头的时间只保留时分，长对话里按时间扫描更快。 */
function formatMessageTime(ms: number): string {
  return new Date(ms).toLocaleTimeString("zh-CN", {
    hour12: false,
    hour: "2-digit",
    minute: "2-digit",
  });
}

/**
 * 消息卡片：最终智能体消息走 Markdown；长消息默认折叠。
 *
 * 折叠只限制可视高度，不截断 DOM，因此展开/复制都保留完整正文。
 */
interface PendingAttachment {
  key: string;
  file: File;
  previewUrl: string | null;
}

interface MessageCardProps {
  message: Message;
  name: string;
  isContinuation: boolean;
  isModerator: boolean;
  moderatorId: number | null;
  agentNameById: ReadonlyMap<number, string>;
  quoted: Message | undefined;
  quotedName: string | null;
  replyToId: number | null;
  mentionNames: readonly string[];
  durationMs: number | null;
  /** 该消息对应的运行 id；刷新后无法恢复，因此可能为 null。 */
  runId: number | null;
  onInspectRun: (runId: number) => void;
  onReply: (message: Message) => void;
  onCopy: (message: Message) => void;
  onToggleReaction: (messageId: number, emoji: string) => void;
  onJumpToMessage: (messageId: number) => void;
  copied: boolean;
  reactionBusy: boolean;
  flash: boolean;
  onLayoutChange: () => void;
}

const MessageAttachmentItem = memo(function MessageAttachmentItem({
  attachment,
}: {
  attachment: MessageAttachment;
}) {
  const [imageFailed, setImageFailed] = useState(false);
  const href = api.attachments.url(attachment.id);
  const isImage = isImageAttachment(attachment.mimeType);

  if (isImage && imageFailed) {
    return (
      <a
        className="msg-attachment-fallback"
        href={href}
        download={attachment.name}
        title={attachment.name}
      >
        图片加载失败：{attachment.name}（{formatFileSize(attachment.size)}）
      </a>
    );
  }

  if (isImage) {
    return (
      <a
        className="msg-attachment-image"
        href={href}
        target="_blank"
        rel="noreferrer"
        title={`${attachment.name}（${formatFileSize(attachment.size)}）`}
      >
        <img
          src={href}
          alt={attachment.name}
          loading="lazy"
          onError={() => setImageFailed(true)}
        />
      </a>
    );
  }

  return (
    <a
      className="msg-attachment-file"
      href={href}
      download={attachment.name}
      title={attachment.name}
    >
      <span className="msg-attachment-name">{attachment.name}</span>
      <span className="msg-attachment-size">
        {formatFileSize(attachment.size)}
      </span>
    </a>
  );
});

function MessageAttachments({
  attachments,
}: {
  attachments: MessageAttachment[];
}) {
  if (attachments.length === 0) {
    return null;
  }
  return (
    <div className="msg-attachments" aria-label="附件">
      {attachments.map((attachment) => (
        <MessageAttachmentItem key={attachment.id} attachment={attachment} />
      ))}
    </div>
  );
}

const MessageCard = memo(function MessageCard({
  message,
  name,
  isContinuation,
  isModerator,
  moderatorId,
  agentNameById,
  quoted,
  quotedName,
  replyToId,
  mentionNames,
  durationMs,
  runId,
  onInspectRun,
  onReply,
  onCopy,
  onToggleReaction,
  onJumpToMessage,
  copied,
  reactionBusy,
  flash,
  onLayoutChange,
}: MessageCardProps) {
  const collapsible = shouldCollapse(message);
  // 长消息在流式阶段还不满足折叠条件；终态后由 collapsible 接管默认折叠。
  const [expanded, setExpanded] = useState(false);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [pickerBelow, setPickerBelow] = useState(false);
  const markdownShellRef = useRef<HTMLDivElement | null>(null);
  const pickerRef = useRef<HTMLDivElement | null>(null);

  /** 顶部空间不足时向下展开，避免浮层被消息滚动容器裁剪。 */
  const togglePicker = () => {
    if (pickerOpen) {
      setPickerOpen(false);
      return;
    }
    const anchor = pickerRef.current?.getBoundingClientRect();
    const list = pickerRef.current
      ?.closest(".message-list")
      ?.getBoundingClientRect();
    const spaceAbove =
      anchor && list ? anchor.top - list.top : Number.POSITIVE_INFINITY;
    setPickerBelow(spaceAbove < 64);
    setPickerOpen(true);
  };

  // 点击外部或按 Esc 关闭表情选择器，避免浮层残留到下次悬停。
  useEffect(() => {
    if (!pickerOpen) {
      return;
    }
    const onPointerDown = (event: PointerEvent) => {
      if (!pickerRef.current?.contains(event.target as Node)) {
        setPickerOpen(false);
      }
    };
    const onKeyDown = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape") {
        setPickerOpen(false);
      }
    };
    document.addEventListener("pointerdown", onPointerDown);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [pickerOpen]);

  // 展开/收起在 DOM 更新后校正底部，避免读到旧高度。
  useEffect(() => {
    onLayoutChange();
  }, [expanded, onLayoutChange]);

  // 懒加载的 Markdown、字体或折叠状态都会改变卡片高度；尺寸变化后重新对齐底部。
  useEffect(() => {
    const shell = markdownShellRef.current;
    if (shell === null) {
      return;
    }
    const observer = new ResizeObserver(() => onLayoutChange());
    observer.observe(shell);
    return () => observer.disconnect();
  }, [onLayoutChange, message.status]);
  const streaming = message.status === "streaming";
  const discussionKind = discussionMessageKind(message, moderatorId);
  const stance = message.discussion?.stance ?? null;
  const stanceLabel = discussionStanceLabel(stance);
  const stopVote = message.discussion?.stopVote ?? null;
  const stopVoteLabel = discussionStopVoteLabel(stopVote);
  const collaborationRole = message.collaboration?.role ?? null;
  const collaborationRoleText =
    collaborationRole === null ? null : collaborationRoleLabel(collaborationRole);
  const interactive = isInteractiveMessage(message);
  const reactions = message.reactions ?? [];
  const mentionSegments =
    message.senderType === "user" && message.kind === "reply"
      ? splitMentions(message.content, mentionNames)
      : null;

  const displayText = streaming
    ? message.content
    : formatDiscussionConclusion(
        formatModeratorTranscript(
          message.content,
          (agentId) => agentNameById.get(agentId),
          isModerator,
        ),
        discussionKind,
      );
  const headings = collapsible ? extractHeadings(displayText) : [];
  const collapsed = collapsible && !expanded;

  return (
    <article
      id={`message-${message.id}`}
      className={`msg msg-${message.senderType} msg-${message.status}${moderatorMessageClass(
        message.senderType,
        isModerator,
      )}${
        message.kind === "progress" ? " is-progress" : ""
      }${
        isContinuation ? " is-continuation" : ""
      }${discussionKind === "judge" ? " is-judge" : ""}${
        collaborationRole === "summary" ? " is-collaboration-summary" : ""
      }${flash ? " is-flash" : ""}`}
    >
      {message.senderType === "user" &&
        (isContinuation ? (
          <span className="avatar avatar-sm avatar-spacer" aria-hidden="true" />
        ) : (
          <span className="avatar avatar-sm avatar-user" aria-hidden="true">
            我
          </span>
        ))}
      {message.senderType === "agent" &&
        (isContinuation ? (
          <span className="avatar avatar-sm avatar-spacer" aria-hidden="true" />
        ) : (
          <AgentAvatar
            id={message.senderAgentId ?? 0}
            name={name}
            size="sm"
          />
        ))}

      <div className="msg-body">
        <header className="msg-head">
          {!isContinuation && <strong>{name}</strong>}
          {message.senderType === "agent" && isModerator && (
            <span className="badge badge-moderator">主持人</span>
          )}
          {discussionKind === "turn" && message.discussion && (
            <>
              <span className="discussion-round">
                第 {message.discussion.round} 轮
              </span>
              {stanceLabel !== null && (
                <span className={`stance-pill stance-${stance}`}>
                  {stanceLabel}
                </span>
              )}
              {stopVoteLabel !== null && (
                <span className={`stance-pill vote-${stopVote}`}>
                  {stopVoteLabel}
                </span>
              )}
            </>
          )}
          {discussionKind === "judge" && (
            <span className="badge badge-discussion">主持人裁决</span>
          )}
          {discussionKind === "system" && (
            <span className="badge badge-discussion">讨论</span>
          )}
          {collaborationRole === "summary" && (
            <span className="badge badge-collaboration-summary">
              {collaborationRoleText}
            </span>
          )}
          {collaborationRole === "system" && (
            <span className="badge badge-collaboration">
              {collaborationRoleText}
            </span>
          )}
          {message.kind === "progress" && (
            <span className="badge badge-progress">阶段总结</span>
          )}
          {streaming && (
            <span className="msg-generating">
              <span className="pulse-dot" aria-hidden="true" />
              生成中
            </span>
          )}
          {message.status === "failed" && (
            <span className="status-pill blocked">失败</span>
          )}
          {durationMs !== null && (
            <span className="msg-duration">耗时 {formatElapsed(durationMs)}</span>
          )}
          {runId !== null && (
            <button
              type="button"
              className="msg-inspect"
              onClick={() => onInspectRun(runId)}
            >
              执行详情
            </button>
          )}
          <time
            className="msg-time"
            dateTime={new Date(message.createdAt).toISOString()}
          >
            {formatMessageTime(message.createdAt)}
          </time>
          {interactive && (
              <div
                className={
                  pickerOpen
                    ? "msg-actions is-open"
                    : "msg-actions"
                }
                role="group"
                aria-label="消息操作"
              >
              <button
                type="button"
                className="msg-action"
                onClick={() => onReply(message)}
              >
                回复
              </button>
              <button
                type="button"
                className="msg-action"
                onClick={() => onCopy(message)}
              >
                {copied ? "已复制" : "复制"}
              </button>
              <div className="msg-action-picker" ref={pickerRef}>
                <button
                  type="button"
                  className="msg-action"
                  aria-expanded={pickerOpen}
                  aria-label="添加表情反应"
                  onClick={togglePicker}
                >
                  表情
                </button>
                {pickerOpen && (
                  <div
                    className={
                      pickerBelow
                        ? "reaction-picker is-below"
                        : "reaction-picker"
                    }
                    role="group"
                    aria-label="表情反应"
                  >
                    {REACTION_EMOJIS.map((emoji) => (
                      <button
                        key={emoji}
                        type="button"
                        className="reaction-picker-item"
                        aria-label={`添加 ${emoji} 反应`}
                        disabled={reactionBusy}
                        onClick={() => {
                          onToggleReaction(message.id, emoji);
                          setPickerOpen(false);
                        }}
                      >
                        {emoji}
                      </button>
                    ))}
                  </div>
                )}
              </div>
            </div>
          )}
        </header>
        {replyToId !== null &&
          (quoted !== undefined ? (
            <button
              type="button"
              className="msg-quote msg-quote-button"
              onClick={() => onJumpToMessage(quoted.id)}
              title="跳转到被回复的消息"
            >
              回复 {quotedName ?? senderNameOf(quoted)}：
              {summarizeQuote(quoted.content)}
            </button>
          ) : (
            <p className="msg-quote">原消息已清理</p>
          ))}
        {message.kind === "reply" &&
        message.senderType === "agent" &&
        message.status === "final" ? (
          <div className="msg-markdown-shell" ref={markdownShellRef}>
            <div
              className={
                collapsed
                  ? "msg-content msg-markdown is-collapsed"
                  : "msg-content msg-markdown"
              }
            >
              <Suspense
                fallback={
                  <p className="msg-markdown-fallback">正在渲染消息…</p>
                }
              >
                <MarkdownContent
                  text={displayText}
                  onRendered={onLayoutChange}
                />
              </Suspense>
            </div>
            {collapsible && (
              <div className="msg-collapse-footer">
                {collapsed && headings.length > 0 && (
                  <ul className="msg-collapse-outline">
                    {headings.map((heading, headingIndex) => (
                      <li key={`${heading}-${headingIndex}`}>{heading}</li>
                    ))}
                  </ul>
                )}
                <button
                  type="button"
                  className="msg-collapse-toggle"
                  aria-expanded={expanded}
                  onClick={() => {
                    setExpanded((value) => !value);
                  }}
                >
                  {collapsed
                    ? `展开全文（共 ${message.content.length} 字）`
                    : "收起"}
                </button>
              </div>
            )}
          </div>
        ) : (
          <p className={message.kind === "progress" ? "msg-content msg-progress-text" : "msg-content"}>
            {message.content === "" && streaming ? (
              "正在思考…"
            ) : mentionSegments !== null ? (
              mentionSegments.map((segment, index) =>
                segment.mention ? (
                  <mark className="mention" key={`${segment.text}-${index}`}>
                    {segment.text}
                  </mark>
                ) : (
                  <span key={`${segment.text}-${index}`}>{segment.text}</span>
                ),
              )
            ) : (
              displayText
            )}
          </p>
        )}
        <MessageAttachments attachments={message.attachments ?? []} />
        {interactive && reactions.length > 0 && (
          <div className="msg-reactions" aria-label="表情反应">
            {reactions.map((reaction) => (
              <button
                key={reaction.emoji}
                type="button"
                className={
                  reaction.reactedByMe
                    ? "msg-reaction is-active"
                    : "msg-reaction"
                }
                aria-pressed={reaction.reactedByMe}
                disabled={reactionBusy}
                onClick={() => onToggleReaction(message.id, reaction.emoji)}
              >
                <span className="msg-reaction-emoji">{reaction.emoji}</span>
                <span className="msg-reaction-count">{reaction.count}</span>
              </button>
            ))}
          </div>
        )}
      </div>
    </article>
  );
});

function senderNameOf(message: Message): string {
  if (message.senderType === "user") {
    return "我";
  }
  if (message.senderType === "system") {
    return "系统";
  }
  return "智能体";
}

/** 消息筛选分组：按钮只切换类别，成员下拉是更细的覆盖条件。 */
const MESSAGE_FILTERS: ReadonlyArray<{ key: MessageFilter; label: string }> = [
  { key: "all", label: "全部" },
  { key: "moderator", label: "主持人" },
  { key: "members", label: "成员" },
  { key: "system", label: "系统" },
];

const PERMISSION_MODE_LABELS: Record<Room["permissionMode"], string> = {
  plan: "计划模式",
  auto: "自主模式",
  full: "完全访问",
};

const PERMISSION_MODE_HINTS: Record<Room["permissionMode"], string> = {
  plan: "只读规划，不写文件",
  auto: "可读写，危险操作受拦截",
  full: "无审批、无沙箱",
};

/**
 * 房间聊天页：历史消息 + 实时增量 + 发送框。
 *
 * 服务端状态（房间/成员/历史消息）通过 API client 获取；
 * 实时增量由 `useRoomEvents` 推入同一份本地消息列表，按 id 合并去重。
 */
export default function RoomPage() {
  const { id } = useParams<{ id: string }>();
  const navigate = useNavigate();
  const roomId =
    id !== undefined && /^\d+$/.test(id) && Number(id) > 0 ? Number(id) : null;
  const { setChrome } = useRoomChrome();

  const [room, setRoom] = useState<Room | null>(null);
  const [agents, setAgents] = useState<Agent[]>([]);
  const [messages, setMessages] = useState<Message[]>([]);
  const [activeRuns, setActiveRuns] = useState<Record<number, ActiveRun>>({});
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [draft, setDraft] = useState("");
  const [cursor, setCursor] = useState(0);
  const [completionIndex, setCompletionIndex] = useState(0);
  const [completionDismissed, setCompletionDismissed] = useState(false);
  const [sending, setSending] = useState(false);
  const [uploading, setUploading] = useState(false);
  const [composerError, setComposerError] = useState<string | null>(null);
  const [pendingAttachments, setPendingAttachments] = useState<
    PendingAttachment[]
  >([]);
  const [attachmentLimits, setAttachmentLimits] = useState<AttachmentLimits>(
    DEFAULT_ATTACHMENT_LIMITS,
  );
  const [discussion, setDiscussion] = useState<Discussion | null>(null);
  const [stoppingDiscussion, setStoppingDiscussion] = useState(false);
  const [resumingDiscussion, setResumingDiscussion] = useState(false);
  const [discussionError, setDiscussionError] = useState<string | null>(null);
  const [collaboration, setCollaboration] =
    useState<CollaborationBatch | null>(null);
  const [stoppingCollaboration, setStoppingCollaboration] = useState(false);
  const [collaborationError, setCollaborationError] = useState<string | null>(
    null,
  );
  const [messageFilter, setMessageFilter] = useState<MessageFilter>("all");
  const [agentFilter, setAgentFilter] = useState<number | null>(null);
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const [permissionSaving, setPermissionSaving] = useState(false);
  const [stickToBottom, setStickToBottom] = useState(true);
  const [unseenCount, setUnseenCount] = useState(0);
  const [runDurations, setRunDurations] = useState<Record<number, number>>({});
  const [runIdByMessageId, setRunIdByMessageId] = useState<
    Record<number, number>
  >({});
  const runIdByMessageIdRef = useRef<Record<number, number>>({});
  const [now, setNow] = useState(() => Date.now());
  const [announcement, setAnnouncement] = useState("");
  const [cleanupOpen, setCleanupOpen] = useState(false);
  const [sessionPanelOpen, setSessionPanelOpen] = useState(false);
  const [keepRecentInput, setKeepRecentInput] = useState("0");
  const [cleaning, setCleaning] = useState(false);
  const [cleanupError, setCleanupError] = useState<string | null>(null);
  const [cleanupResult, setCleanupResult] = useState<string | null>(null);
  const [contextPreview, setContextPreview] = useState<ContextPreview | null>(
    null,
  );
  const [contextLoading, setContextLoading] = useState(false);
  const [contextRefreshKey, setContextRefreshKey] = useState(0);
  const [queue, setQueue] = useState<QueueSnapshot | null>(null);
  const [queueError, setQueueError] = useState(false);
  const [queueNotice, setQueueNotice] = useState<string | null>(null);
  const [cancellingQueueJobId, setCancellingQueueJobId] = useState<
    number | null
  >(null);
  const [queuePauseBusy, setQueuePauseBusy] = useState(false);
  const [queueRefreshKey, setQueueRefreshKey] = useState(0);
  const [files, setFiles] = useState<RoomFiles | null>(null);
  const [filesError, setFilesError] = useState<string | null>(null);
  const [filesRefreshKey, setFilesRefreshKey] = useState(0);
  const [sideTab, setSideTab] = useState<RoomSideTab>(() => readRoomSideTab());
  const [replyTo, setReplyTo] = useState<Message | null>(null);
  const [copiedMessageId, setCopiedMessageId] = useState<number | null>(null);
  const [reactionBusyId, setReactionBusyId] = useState<number | null>(null);
  const [flashMessageId, setFlashMessageId] = useState<number | null>(null);
  const [messageNotice, setMessageNotice] = useState<string | null>(null);

  const messageListRef = useRef<HTMLDivElement | null>(null);
  const stickRef = useRef(true);
  const activeRunsRef = useRef<Record<number, ActiveRun>>({});
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
  const cleanupInputRef = useRef<HTMLInputElement | null>(null);
  const sideCloseRef = useRef<HTMLButtonElement | null>(null);
  const attachmentInputRef = useRef<HTMLInputElement | null>(null);
  const attachmentKeyRef = useRef(0);
  const pendingAttachmentsRef = useRef<PendingAttachment[]>([]);
  const messagesRef = useRef<Message[]>([]);
  const reactionBusyRef = useRef<number | null>(null);

  useEffect(() => {
    messagesRef.current = messages;
  }, [messages]);

  // 复制与跳转高亮都是短时反馈，计时器随状态清理。
  useEffect(() => {
    if (copiedMessageId === null) {
      return;
    }
    const timer = window.setTimeout(() => setCopiedMessageId(null), 1500);
    return () => window.clearTimeout(timer);
  }, [copiedMessageId]);

  useEffect(() => {
    if (flashMessageId === null) {
      return;
    }
    const timer = window.setTimeout(() => setFlashMessageId(null), 1200);
    return () => window.clearTimeout(timer);
  }, [flashMessageId]);

  useEffect(() => {
    pendingAttachmentsRef.current = pendingAttachments;
  }, [pendingAttachments]);

  // 离开房间时释放仍未发送的图片预览 URL。
  useEffect(() => {
    return () => {
      for (const attachment of pendingAttachmentsRef.current) {
        if (attachment.previewUrl !== null) {
          URL.revokeObjectURL(attachment.previewUrl);
        }
      }
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    api.settings
      .get()
      .then((response) => {
        if (!cancelled) {
          setAttachmentLimits({
            maxFileBytes: response.settings.attachmentMaxFileBytes,
            maxFiles: response.settings.attachmentMaxFilesPerMessage,
          });
        }
      })
      .catch(() => {
        // 设置读取失败时沿用默认限制，服务端仍会再次校验。
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // 房间名与返回入口收进应用顶栏，页内不再渲染大标题。
  useEffect(() => {
    setChrome(buildRoomChrome(room, roomId));
    return () => setChrome(null);
  }, [room, roomId, setChrome]);

  // 侧栏默认收起；打开时把焦点移入浮层，Esc 关闭。
  useEffect(() => {
    if (!sidebarOpen) {
      return;
    }
    sideCloseRef.current?.focus();
    const closeOnEscape = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape") {
        setSidebarOpen(false);
      }
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [sidebarOpen]);

  useEffect(() => {
    if (roomId === null) {
      setLoading(false);
      setError("非法的房间地址");
      setRoom(null);
      setMessages([]);
      return;
    }

    let cancelled = false;
    setLoading(true);
    setError(null);
    setRoom(null);
    setMessages([]);
    setActiveRuns({});
    activeRunsRef.current = {};
    setRunDurations({});
    setRunIdByMessageId({});
    runIdByMessageIdRef.current = {};
    setCleanupOpen(false);
    setKeepRecentInput("0");
    setCleaning(false);
    setCleanupError(null);
    setCleanupResult(null);
    setContextPreview(null);
    setQueue(null);
    setQueueError(false);
    setQueueNotice(null);
    setCancellingQueueJobId(null);
    setQueuePauseBusy(false);
    setReplyTo(null);
    setCopiedMessageId(null);
    setReactionBusyId(null);
    setFlashMessageId(null);
    setMessageNotice(null);
    setDiscussion(null);
    setDiscussionError(null);
    setCollaboration(null);
    setCollaborationError(null);
    setStoppingCollaboration(false);
    setMessageFilter("all");
    setAgentFilter(null);
    setUnseenCount(0);
    setAnnouncement("");
    stickRef.current = true;
    setStickToBottom(true);

    void (async () => {
      try {
        const [
          roomList,
          agentList,
          history,
          latestDiscussion,
          latestCollaboration,
        ] = await Promise.all([
          api.rooms.list(),
          api.agents.list(),
          api.messages.list(roomId),
          api.discussions.latest(roomId),
          api.collaborations.latest(roomId),
        ]);
        if (cancelled) {
          return;
        }
        setAgents(agentList);
        setRoom(roomList.find((candidate) => candidate.id === roomId) ?? null);
        setDiscussion(latestDiscussion);
        // 快照可能晚于已到达的 WS 事件；只在它更新时覆盖，避免状态回退。
        setCollaboration((previous) =>
          latestCollaboration === null
            ? previous
            : previous === null ||
                latestCollaboration.updatedAt >= previous.updatedAt
              ? latestCollaboration
              : previous,
        );
        // 历史快照与已到达的实时消息合并，避免覆盖流式增量/终态。
        setMessages((previous) => mergeMessages(previous, history));
      } catch (cause) {
        if (!cancelled) {
          setError(errorMessage(cause));
        }
      } finally {
        if (!cancelled) {
          setLoading(false);
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [roomId]);

  /** 按 id 合并：已存在则按状态优先级取更新的一份，否则按 seq 插入。 */
  const mergeMessage = useCallback((incoming: Message) => {
    setMessages((previous) => mergeMessages(previous, [incoming]));
  }, []);

  /** 历史清理后整体重拉；清理期间房间被判定为空闲，不存在流式消息。 */
  const reloadMessages = useCallback(async () => {
    if (roomId === null) {
      return;
    }
    try {
      setMessages(await api.messages.list(roomId));
    } catch (cause) {
      setError(errorMessage(cause));
    }
  }, [roomId]);

  const handleEvent = useCallback(
    (event: RoomEvent) => {
      // 丢弃跨房间的过期事件：它们不属于当前页面展示的消息列表。
      if (!isEventForRoom(event, roomId)) {
        return;
      }

      // 任务状态变化由任务页展示，聊天面板不消费该事件。
      if (event.type === "task_changed") {
        return;
      }

      // 讨论状态条只消费快照；讨论消息本身仍走 message_final。
      if (event.type === "discussion_changed") {
        setDiscussion(event.discussion);
        return;
      }

      // 委派批次同样只消费快照；按 updatedAt 丢弃迟到的旧广播。
      if (event.type === "collaboration_changed") {
        setCollaboration((previous) =>
          previous === null || event.batch.updatedAt >= previous.updatedAt
            ? event.batch
            : previous,
        );
        return;
      }

      // 运行活动由独立运行详情页消费，房间页不保留活动状态。
      if (event.type === "run_activity") {
        return;
      }

      // 纯聊天历史已清理（可能来自其它标签页），整体重拉列表。
      if (event.type === "history_cleared") {
        setContextRefreshKey((key) => key + 1);
        setQueueRefreshKey((key) => key + 1);
        setFilesRefreshKey((key) => key + 1);
        void reloadMessages();
        return;
      }

      // 反应只替换目标消息的 reactions，不动正文与运行状态。
      if (event.type === "message_reactions_changed") {
        setMessages((previous) =>
          previous.map((item) =>
            item.id === event.messageId
              ? { ...item, reactions: event.reactions }
              : item,
          ),
        );
        return;
      }

      if (event.type === "run_started") {
        const run: ActiveRun = {
          runId: event.runId,
          agentId: event.agentId,
          messageId: event.messageId,
          startedAt: Date.now(),
        };
        activeRunsRef.current = {
          ...activeRunsRef.current,
          [event.runId]: run,
        };
        setActiveRuns((previous) => ({
          ...previous,
          [event.runId]: run,
        }));
        setRunIdByMessageId((previous) => ({
          ...previous,
          [event.messageId]: event.runId,
        }));
        runIdByMessageIdRef.current = {
          ...runIdByMessageIdRef.current,
          [event.messageId]: event.runId,
        };
        setAnnouncement("成员开始回复");
        setQueueRefreshKey((key) => key + 1);
        mergeMessage(event.message);
        return;
      }

      // 同一运行进入新阶段：登记新消息，运行状态保持不变。
      if (event.type === "message_started") {
        setRunIdByMessageId((previous) => ({
          ...previous,
          [event.message.id]: event.runId,
        }));
        runIdByMessageIdRef.current = {
          ...runIdByMessageIdRef.current,
          [event.message.id]: event.runId,
        };
        mergeMessage(event.message);
        return;
      }

      if (event.type === "message_delta") {
        setMessages((previous) =>
          previous.map((item) =>
            item.id === event.messageId
              ? { ...item, content: item.content + event.text }
              : item,
          ),
        );
        return;
      }

      // message_final：progress 只落定阶段消息；reply 才结束整个运行。
      const finishedRunId =
        event.message.runId ?? runIdByMessageIdRef.current[event.message.id] ?? null;
      const finishedRun =
        finishedRunId === null ? undefined : activeRunsRef.current[finishedRunId];
      if (finishedRun !== undefined && event.message.kind === "reply") {
        const nextRuns = { ...activeRunsRef.current };
        delete nextRuns[finishedRunId!];
        activeRunsRef.current = nextRuns;
        setRunDurations((previous) => ({
          ...previous,
          [finishedRunId!]: Date.now() - finishedRun.startedAt,
        }));
        setActiveRuns((previous) => {
          if (!(finishedRunId! in previous)) {
            return previous;
          }
          const next = { ...previous };
          delete next[finishedRunId!];
          return next;
        });
      }
      if (event.message.kind === "reply") {
        // 用户/系统/成员的任何 final reply 都会进入历史，统一刷新估算。
        setContextRefreshKey((key) => key + 1);
        setQueueRefreshKey((key) => key + 1);
        setFilesRefreshKey((key) => key + 1);
        if (event.message.senderType === "agent") {
          setAnnouncement("成员回复已完成");
          if (!stickRef.current) {
            setUnseenCount((count) => count + 1);
          }
        }
      }
      mergeMessage(event.message);
    },
    [mergeMessage, reloadMessages, roomId],
  );

  const { connected } = useRoomEvents(roomId, handleEvent);

  const runningCount = Object.keys(activeRuns).length;

  /** 房间内直接切换权限模式；保存成功后用服务端结果回填。 */
  const changePermissionMode = async (mode: Room["permissionMode"]) => {
    if (room === null || room.permissionMode === mode || permissionSaving) {
      return;
    }
    setPermissionSaving(true);
    setError(null);
    try {
      setRoom(await api.rooms.update(room.id, { permissionMode: mode }));
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setPermissionSaving(false);
    }
  };

  /** 打开运行详情页；不带 runId 时进入运行列表。 */
  const openRunPage = useCallback(
    (runId?: number) => {
      if (roomId === null) {
        return;
      }
      navigate(
        runId === undefined
          ? `/rooms/${roomId}/runs`
          : `/rooms/${roomId}/runs/${runId}`,
      );
    },
    [navigate, roomId],
  );

  const openCleanup = useCallback(() => {
    setKeepRecentInput("0");
    setCleanupError(null);
    setCleanupResult(null);
    setCleanupOpen(true);
  }, []);

  const closeCleanup = useCallback(() => {
    if (cleaning) {
      return;
    }
    setCleanupOpen(false);
    setCleanupError(null);
  }, [cleaning]);

  const preview = useMemo(
    () => cleanupPreview(messages, parseKeepRecent(keepRecentInput) ?? 0),
    [messages, keepRecentInput],
  );

  // 模态框打开时聚焦输入并支持 Escape 关闭。
  useEffect(() => {
    if (!cleanupOpen) {
      return;
    }
    cleanupInputRef.current?.focus();
    const handleKeyDown = (event: globalThis.KeyboardEvent): void => {
      if (event.key === "Escape") {
        closeCleanup();
      }
    };
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [cleanupOpen, closeCleanup]);

  const submitCleanup = useCallback(async () => {
    if (roomId === null) {
      return;
    }
    const keepRecent = parseKeepRecent(keepRecentInput);
    if (keepRecent === null) {
      setCleanupError("请输入 0 到 5000 之间的整数");
      return;
    }
    setCleaning(true);
    setCleanupError(null);
    try {
      const result = await api.messages.cleanup(roomId, keepRecent);
      setCleanupResult(formatCleanupResult(result));
      setCleanupOpen(false);
      await reloadMessages();
    } catch (cause) {
      setCleanupError(errorMessage(cause));
    } finally {
      setCleaning(false);
    }
  }, [keepRecentInput, reloadMessages, roomId]);

  const scrollToBottom = useCallback(
    (behavior: ScrollBehavior = "auto") => {
      const list = messageListRef.current;
      if (list === null) {
        return;
      }
      if (behavior === "smooth") {
        list.scrollTo({ top: list.scrollHeight, behavior });
        return;
      }
      list.scrollTop = list.scrollHeight;
    },
    [],
  );

  // Markdown chunk 加载完成后内容高度会变化；若仍贴底，再滚一次。
  useEffect(() => {
    let active = true;
    void import("../components/MarkdownContent").then(() => {
      if (!active || !stickRef.current) {
        return;
      }
      scrollToBottom();
    });
    return () => {
      active = false;
    };
  }, [scrollToBottom]);

  // messages 每次变化都代表内容高度可能变化；只有用户仍在底部时才跟随。
  useLayoutEffect(() => {
    if (!stickRef.current) {
      return;
    }
    scrollToBottom();
  }, [messages, stickToBottom, scrollToBottom]);

  // 切换筛选后回到最新，避免停留在被过滤掉的旧位置。
  useEffect(() => {
    stickRef.current = true;
    setStickToBottom(true);
    setUnseenCount(0);
    scrollToBottom();
  }, [messageFilter, agentFilter, scrollToBottom]);

  useEffect(() => {
    if (runningCount === 0) {
      return;
    }
    setNow(Date.now());
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [runningCount]);

  // 运行状态条出现或消失会改变列表可视高度；仍在底部时重新贴底。
  useEffect(() => {
    if (!stickRef.current) {
      return;
    }
    scrollToBottom();
  }, [runningCount, scrollToBottom]);

  const handleMessageListScroll = () => {
    const list = messageListRef.current;
    if (list === null) {
      return;
    }
    const distance = list.scrollHeight - list.scrollTop - list.clientHeight;
    const nextStick = distance < 80;
    stickRef.current = nextStick;
    setStickToBottom(nextStick);
    if (nextStick) {
      setUnseenCount(0);
    }
  };

  const jumpToLatest = () => {
    stickRef.current = true;
    setStickToBottom(true);
    setUnseenCount(0);
    scrollToBottom("smooth");
  };

  const handleMessageLayoutChange = useCallback(() => {
    if (!stickRef.current) {
      return;
    }
    scrollToBottom();
  }, [scrollToBottom]);

  const agentNameById = useMemo(() => {
    const map = new Map<number, string>();
    for (const agent of agents) {
      map.set(agent.id, agent.name);
    }
    return map;
  }, [agents]);

  const senderName = (message: Message): string => {
    if (message.senderType === "user") {
      return "我";
    }
    if (message.senderType === "system") {
      return "系统";
    }
    if (message.senderAgentId === null) {
      return "智能体";
    }
    return agentNameById.get(message.senderAgentId) ?? `智能体#${message.senderAgentId}`;
  };

  const messageById = useMemo(() => {
    const map = new Map<number, Message>();
    for (const message of messages) {
      map.set(message.id, message);
    }
    return map;
  }, [messages]);

  const discussionRunning = discussion?.status === "running";

  const clearPendingAttachments = useCallback(() => {
    for (const attachment of pendingAttachmentsRef.current) {
      if (attachment.previewUrl !== null) {
        URL.revokeObjectURL(attachment.previewUrl);
      }
    }
    setPendingAttachments([]);
  }, []);

  const addAttachments = useCallback(
    (files: FileList | File[]) => {
      if (sending || discussionRunning) {
        return;
      }
      const { accepted, errors } = filterPendingFiles(
        pendingAttachmentsRef.current.length,
        Array.from(files),
        attachmentLimits,
      );
      setComposerError(errors.length > 0 ? errors.join("；") : null);
      if (accepted.length === 0) {
        return;
      }
      const next = accepted.map((file) => {
        attachmentKeyRef.current += 1;
        return {
          key: `${Date.now()}-${attachmentKeyRef.current}`,
          file,
          previewUrl: isImageAttachment(file.type)
            ? URL.createObjectURL(file)
            : null,
        } satisfies PendingAttachment;
      });
      setPendingAttachments((current) => [...current, ...next]);
    },
    [attachmentLimits, discussionRunning, sending],
  );

  const removePendingAttachment = (key: string) => {
    const target = pendingAttachmentsRef.current.find(
      (attachment) => attachment.key === key,
    );
    if (target?.previewUrl != null) {
      URL.revokeObjectURL(target.previewUrl);
    }
    setPendingAttachments((current) =>
      current.filter((attachment) => attachment.key !== key),
    );
  };

  const handleComposerPaste = (event: ClipboardEvent<HTMLTextAreaElement>) => {
    const files = event.clipboardData.files;
    if (files.length > 0) {
      event.preventDefault();
      addAttachments(files);
    }
  };

  const handleComposerDrop = (event: DragEvent<HTMLFormElement>) => {
    if (event.dataTransfer.files.length === 0) {
      return;
    }
    event.preventDefault();
    addAttachments(event.dataTransfer.files);
  };
  const discussionParticipantNames =
    discussion === null
      ? ""
      : formatParticipantNames(discussion.participantIds, (agentId) =>
          agentNameById.get(agentId),
        );
  const discussionStopReason =
    discussion === null ? null : discussionStopReasonLabel(discussion.stopReason);
  const collaborationRunning = collaboration?.status === "running";
  const collaborationRoundAssignments =
    collaboration === null
      ? []
      : assignmentsForRound(collaboration.assignments, collaboration.currentRound);
  const collaborationCounts = countAssignments(collaborationRoundAssignments);
  const collaborationStopReason =
    collaboration === null
      ? null
      : collaborationStopReasonLabel(collaboration.stopReason);
  const collaborationSummary = collaboration?.summary?.trim() ?? "";

  /** 停止是检查点语义：当前成员与汇总运行结束后不再派发下一轮。 */
  const stopCollaboration = async (): Promise<void> => {
    if (
      roomId === null ||
      collaboration === null ||
      collaboration.status !== "running" ||
      collaboration.stopRequested ||
      stoppingCollaboration
    ) {
      return;
    }

    setStoppingCollaboration(true);
    setCollaborationError(null);
    try {
      const updated = await api.collaborations.stop(roomId, collaboration.id);
      setCollaboration(updated);
    } catch (cause) {
      setCollaborationError(errorMessage(cause));
    } finally {
      setStoppingCollaboration(false);
    }
  };

  /** 停止是检查点语义：服务端在本次发言结束后不再派发下一轮。 */
  const stopDiscussion = async (): Promise<void> => {
    if (
      roomId === null ||
      discussion === null ||
      discussion.status !== "running" ||
      stoppingDiscussion
    ) {
      return;
    }

    setStoppingDiscussion(true);
    setDiscussionError(null);
    try {
      const updated = await api.discussions.stop(roomId, discussion.id);
      setDiscussion(updated);
    } catch (cause) {
      setDiscussionError(errorMessage(cause));
    } finally {
      setStoppingDiscussion(false);
    }
  };

  /** 续跑失败讨论：复用同一讨论从断点继续，不新建讨论。 */
  const resumeDiscussion = async (): Promise<void> => {
    if (
      roomId === null ||
      discussion === null ||
      !isResumableDiscussion(discussion.status) ||
      resumingDiscussion
    ) {
      return;
    }

    setResumingDiscussion(true);
    setDiscussionError(null);
    try {
      const updated = await api.discussions.resume(roomId, discussion.id);
      setDiscussion(updated);
    } catch (cause) {
      setDiscussionError(errorMessage(cause));
    } finally {
      setResumingDiscussion(false);
    }
  };

  const members = useMemo<MemberView[]>(    () =>
      (room?.memberIds ?? [])
        .map((agentId) => {
          const agent = agents.find((candidate) => candidate.id === agentId);
          return {
            id: agentId,
            name: agent?.name ?? agentNameById.get(agentId) ?? `#${agentId}`,
            builtinRole: agent?.builtinRole ?? null,
          };
        })
        .sort(
          (left, right) =>
            Number(right.builtinRole === "moderator") -
            Number(left.builtinRole === "moderator"),
        ),
    [room, agents, agentNameById],
  );
  const mentionNames = useMemo(
    () => members.map((member) => member.name),
    [members],
  );
  const builtinModerator =
    members.find((member) => member.builtinRole === "moderator") ?? null;
  // 上下文估算优先用内置主持人；没有主持人时用第一位成员。
  const contextAgentId = builtinModerator?.id ?? members[0]?.id ?? null;

  useEffect(() => {
    if (roomId === null || contextAgentId === null) {
      setContextPreview(null);
      setContextLoading(false);
      return;
    }
    let cancelled = false;
    setContextLoading(true);
    api.rooms
      .contextPreview(roomId, contextAgentId)
      .then((preview) => {
        if (!cancelled) {
          setContextPreview(preview);
        }
      })
      .catch(() => {
        // 预览失败静默隐藏，不打断聊天。
        if (!cancelled) {
          setContextPreview(null);
        }
      })
      .finally(() => {
        if (!cancelled) {
          setContextLoading(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [roomId, contextAgentId, contextRefreshKey]);

  // 队列快照由服务端轮询；发送/运行事件后立即刷新。
  useEffect(() => {
    if (roomId === null) {
      setQueue(null);
      return;
    }
    let cancelled = false;
    const load = async () => {
      try {
        const snapshot = await api.rooms.queue(roomId);
        if (!cancelled) {
          setQueue(snapshot);
          setQueueError(false);
        }
      } catch {
        // 刷新失败保留上次快照，侧栏显示小字提示。
        if (!cancelled) {
          setQueueError(true);
        }
      }
    };
    void load();
    const timer = window.setInterval(() => {
      void load();
    }, 2500);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [roomId, queueRefreshKey]);

  // 文件清单按需刷新：进入房间、成员回复完成或点击刷新时重新读取。
  useEffect(() => {
    if (roomId === null) {
      setFiles(null);
      return;
    }
    let cancelled = false;
    const load = async () => {
      try {
        const snapshot = await api.rooms.files(roomId);
        if (!cancelled) {
          setFiles(snapshot);
          setFilesError(null);
        }
      } catch (cause) {
        if (!cancelled) {
          setFilesError(errorMessage(cause));
        }
      }
    };
    void load();
    return () => {
      cancelled = true;
    };
  }, [roomId, filesRefreshKey]);

  useEffect(() => {
    persistRoomSideTab(sideTab);
  }, [sideTab]);

  const cancelQueueJob = async (jobId: number) => {
    if (roomId === null || cancellingQueueJobId !== null) {
      return;
    }
    setCancellingQueueJobId(jobId);
    setQueueNotice(null);
    try {
      await api.rooms.cancelQueueJob(roomId, jobId);
      setQueueRefreshKey((key) => key + 1);
    } catch (cause) {
      // 409：任务已开跑或类型不可取消；保留可见提示。
      setQueueNotice(errorMessage(cause));
    } finally {
      setCancellingQueueJobId(null);
    }
  };

  const toggleQueuePaused = async () => {
    if (roomId === null || queue === null || queuePauseBusy) {
      return;
    }
    setQueuePauseBusy(true);
    setQueueNotice(null);
    try {
      const snapshot = await api.rooms.setQueuePaused(roomId, !queue.paused);
      setQueue(snapshot);
      setQueueError(false);
    } catch (cause) {
      setQueueNotice(errorMessage(cause));
    } finally {
      setQueuePauseBusy(false);
    }
  };

  const focusSideTab = (tab: RoomSideTab) => {
    setSideTab(tab);
    window.requestAnimationFrame(() => {
      document.getElementById(`room-side-tab-${tab}`)?.focus();
    });
  };

  const onSideTabKeyDown = (event: KeyboardEvent<HTMLButtonElement>) => {
    const next = nextRoomSideTab(sideTab, event.key);
    if (next !== null) {
      event.preventDefault();
      focusSideTab(next);
    }
  };

  const startReply = useCallback((message: Message) => {
    setReplyTo(message);
    setMessageNotice(null);
    textareaRef.current?.focus();
  }, []);

  const copyMessage = useCallback(async (message: Message) => {
    try {
      await navigator.clipboard.writeText(message.content);
      setCopiedMessageId(message.id);
      setMessageNotice(null);
    } catch {
      setMessageNotice("复制失败，请手动选择文本");
    }
  }, []);

  const toggleReaction = useCallback(
    async (messageId: number, emoji: string) => {
      if (roomId === null || reactionBusyRef.current !== null) {
        return;
      }
      const previous =
        messagesRef.current.find((message) => message.id === messageId)
          ?.reactions ?? [];
      // 先本地切换保证点击即时反馈，失败时回滚到服务端旧值。
      setMessages((current) =>
        current.map((message) =>
          message.id === messageId
            ? { ...message, reactions: applyReactionToggle(previous, emoji) }
            : message,
        ),
      );
      reactionBusyRef.current = messageId;
      setReactionBusyId(messageId);
      setMessageNotice(null);
      try {
        const { reactions } = await api.messages.toggleReaction(
          roomId,
          messageId,
          emoji,
        );
        setMessages((current) =>
          current.map((message) =>
            message.id === messageId
              ? { ...message, reactions }
              : message,
          ),
        );
      } catch (cause) {
        setMessages((current) =>
          current.map((message) =>
            message.id === messageId
              ? { ...message, reactions: previous }
              : message,
          ),
        );
        setMessageNotice(errorMessage(cause));
      } finally {
        reactionBusyRef.current = null;
        setReactionBusyId(null);
      }
    },
    [roomId],
  );

  const jumpToMessage = useCallback((messageId: number) => {
    const target = document.getElementById(`message-${messageId}`);
    if (target === null) {
      setMessageNotice("原消息不在当前筛选结果中");
      return;
    }
    target.scrollIntoView({ behavior: "smooth", block: "center" });
    setFlashMessageId(messageId);
  }, []);

  const runningRuns = useMemo(
    () =>
      Object.values(activeRuns).map((run) => ({
        ...run,
        name: agentNameById.get(run.agentId) ?? `#${run.agentId}`,
        elapsedMs: Math.max(0, now - run.startedAt),
      })),
    [activeRuns, agentNameById, now],
  );

  const visibleMessages = useMemo(
    () =>
      filterMessages(messages, messageFilter, {
        moderatorId: builtinModerator?.id ?? null,
        agentId: agentFilter,
      }),
    [messages, messageFilter, agentFilter, builtinModerator],
  );
  const groupedMessages = useMemo(
    () => groupMessages(visibleMessages),
    [visibleMessages],
  );

  const trigger = useMemo(
    () => findCompletionTrigger(draft, cursor),
    [draft, cursor],
  );

  const completionOptions = useMemo<CompletionOption[]>(() => {
    if (trigger === null) {
      return [];
    }
    const query = trigger.query.trim().toLowerCase();

    if (trigger.kind === "mention") {
      return members
        .filter((member) => member.name.toLowerCase().includes(query))
        .map((member) => ({
          key: `mention-${member.id}`,
          label: `@${member.name}`,
          description:
            member.builtinRole === "moderator"
              ? "内置主持人 · 自动接管或编排"
              : "房间成员",
          insert: `@${member.name} `,
        }));
    }

    return MODERATOR_COMMANDS.filter(
      (command) =>
        command.name.toLowerCase().includes(query) ||
        command.description.toLowerCase().includes(query),
    ).map((command) => ({
      key: `command-${command.name}`,
      label: `/${command.name}`,
      description: command.description,
      insert: command.template,
    }));
  }, [trigger, members]);

  const completionOpen =
    trigger !== null && !completionDismissed && completionOptions.length > 0;
  const activeCompletionIndex = Math.min(
    completionIndex,
    Math.max(0, completionOptions.length - 1),
  );

  const commitDraft = (value: string, nextCursor: number) => {
    setDraft(value);
    setCursor(nextCursor);
    setCompletionDismissed(false);
    setCompletionIndex(0);
    requestAnimationFrame(() => {
      const textarea = textareaRef.current;
      if (textarea !== null) {
        textarea.focus();
        textarea.setSelectionRange(nextCursor, nextCursor);
      }
    });
  };

  const insertMention = (name: string) => {
    const next =
      draft === ""
        ? `@${name} `
        : `${draft.replace(/\s*$/, " ")}@${name} `;
    commitDraft(next, next.length);
  };

  const applyCompletion = (option: CompletionOption) => {
    if (trigger === null) {
      return;
    }
    const next = `${draft.slice(0, trigger.start)}${option.insert}${draft.slice(trigger.end)}`;
    commitDraft(next, trigger.start + option.insert.length);
  };

  const handleComposerChange = (
    event: ChangeEvent<HTMLTextAreaElement>,
  ): void => {
    setDraft(event.target.value);
    setCursor(event.target.selectionStart);
    setCompletionDismissed(false);
    setCompletionIndex(0);
  };

  const handleComposerKeyDown = (
    event: KeyboardEvent<HTMLTextAreaElement>,
  ): void => {
    const keyAction = composerKeyAction({
      key: event.key,
      shiftKey: event.shiftKey,
      isComposing: event.nativeEvent.isComposing,
      completionOpen,
    });
    if (keyAction === "send") {
      event.preventDefault();
      event.currentTarget.form?.requestSubmit();
      return;
    }
    if (keyAction === "completion") {
      event.preventDefault();
      const option = completionOptions[activeCompletionIndex];
      if (option !== undefined) {
        applyCompletion(option);
      }
      return;
    }
    if (keyAction === "newline" || !completionOpen) {
      return;
    }

    if (event.key === "ArrowDown") {
      event.preventDefault();
      setCompletionIndex((current) =>
        (current + 1) % completionOptions.length,
      );
      return;
    }
    if (event.key === "ArrowUp") {
      event.preventDefault();
      setCompletionIndex(
        (current) =>
          (current - 1 + completionOptions.length) %
          completionOptions.length,
      );
      return;
    }
    if (event.key === "Tab") {
      event.preventDefault();
      const option = completionOptions[activeCompletionIndex];
      if (option !== undefined) {
        applyCompletion(option);
      }
      return;
    }
    if (event.key === "Escape") {
      event.preventDefault();
      setCompletionDismissed(true);
    }
  };

  const send = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (roomId === null || sending || discussionRunning) {
      return;
    }
    const content = draft.trim();
    const attachments = pendingAttachmentsRef.current;
    if (content === "" && attachments.length === 0) {
      return;
    }

    setSending(true);
    setUploading(attachments.length > 0);
    setComposerError(null);
    const uploadedIds: number[] = [];
    try {
      for (const attachment of attachments) {
        const uploaded = await api.attachments.upload(
          roomId,
          attachment.file,
        );
        uploadedIds.push(uploaded.id);
      }
      const { userMessage } = await api.messages.send(
        roomId,
        content,
        uploadedIds,
        replyTo?.id ?? null,
      );
      mergeMessage(userMessage);
      setQueueRefreshKey((key) => key + 1);
      setDraft("");
      setReplyTo(null);
      clearPendingAttachments();
    } catch (cause) {
      // 消息未发送成功时，删除本次已上传但未绑定的附件，避免孤儿文件。
      await Promise.allSettled(
        uploadedIds.map((id) => api.attachments.remove(id)),
      );
      setComposerError(errorMessage(cause));
    } finally {
      setUploading(false);
      setSending(false);
    }
  };

  if (loading) {
    return (
      <section className="page">
        <p className="muted">加载中…</p>
      </section>
    );
  }

  if (error !== null || roomId === null) {
    return (
      <section className="page">
        <p className="banner error" role="alert">
          {error ?? "非法的房间地址"}
        </p>
        <Link className="back-link" to="/rooms">
          返回房间列表
        </Link>
      </section>
    );
  }

  return (
    <section className="page room-page page-fixed">
      <p className="sr-only" role="status" aria-live="polite">
        {announcement}
      </p>

      <div className={`room-layout${sidebarOpen ? " side-open" : ""}`}>
        <div className="room-main">
          <div
            className="room-rail"
            role="toolbar"
            aria-label="房间操作"
            aria-orientation="vertical"
          >
            <button
              type="button"
              className="room-rail-button"
              title="运行记录"
              aria-label="运行记录"
              onClick={() => openRunPage()}
            >
              <HistoryIcon />
            </button>
            <button
              type="button"
              className="room-rail-button"
              title="会话"
              aria-label="会话"
              onClick={() => setSessionPanelOpen(true)}
            >
              <SessionsIcon />
            </button>
            <button
              type="button"
              className="room-rail-button is-danger"
              title="清理历史"
              aria-label="清理历史"
              onClick={openCleanup}
            >
              <TrashIcon />
            </button>
            <button
              type="button"
              className="room-rail-button"
              title={sidebarOpen ? "收起侧栏" : "展开侧栏"}
              aria-label={sidebarOpen ? "收起侧栏" : "展开侧栏"}
              aria-expanded={sidebarOpen}
              aria-controls="room-side"
              onClick={() => setSidebarOpen((open) => !open)}
            >
              <PanelIcon />
            </button>
          </div>

          {discussionError !== null && (
            <p className="banner error" role="alert">
              {discussionError}
            </p>
          )}

          {collaborationError !== null && (
            <p className="banner error" role="alert">
              {collaborationError}
            </p>
          )}

      <div className="chat-shell">
        <div className="chat-panel">
          <div className="message-filter" role="toolbar" aria-label="消息筛选">
            {MESSAGE_FILTERS.map((filter) => {
              const active = messageFilter === filter.key && agentFilter === null;
              return (
                <button
                  key={filter.key}
                  type="button"
                  className={active ? "filter-chip is-active" : "filter-chip"}
                  aria-pressed={active}
                  onClick={() => {
                    setMessageFilter(filter.key);
                    setAgentFilter(null);
                  }}
                >
                  {filter.label}
                </button>
              );
            })}
            <label className="message-filter-agent">
              <span className="sr-only">按成员筛选</span>
              <select
                value={agentFilter === null ? "" : String(agentFilter)}
                onChange={(event) => {
                  const value = event.target.value;
                  if (value === "") {
                    setAgentFilter(null);
                    return;
                  }
                  setAgentFilter(Number(value));
                  setMessageFilter("all");
                }}
              >
                <option value="">全部成员</option>
                {members.map((member) => (
                  <option key={member.id} value={member.id}>
                    @{member.name}
                  </option>
                ))}
              </select>
            </label>
          </div>

          <div
            className="message-list"
            ref={messageListRef}
            onScroll={handleMessageListScroll}
          >
            {messages.length === 0 ? (
              <div className="empty-state">
                <p>
                  {room?.replyMode === "moderator"
                    ? "直接发送消息即可：内置主持人会判断直接回答、委派成员或创建任务；输入 @ 可指定成员。"
                    : "还没有消息。输入「@成员名 + 内容」即可让智能体在该房间工作目录中开工；需要讨论时请 @主持人 说明主题。"}
                </p>
              </div>
            ) : groupedMessages.length === 0 ? (
              <div className="empty-state">
                <p>当前筛选没有消息。</p>
                <button
                  type="button"
                  className="btn btn-secondary btn-sm"
                  onClick={() => {
                    setMessageFilter("all");
                    setAgentFilter(null);
                  }}
                >
                  清除筛选
                </button>
              </div>
            ) : (
              groupedMessages.map(({ message, isContinuation }, index) => {
                const name = senderName(message);
                const replyToId =
                  message.replyToMessageId ??
                  message.discussion?.replyToMessageId ??
                  null;
                const quoted =
                  replyToId === null ? undefined : messageById.get(replyToId);
                const previous =
                  index === 0 ? null : groupedMessages[index - 1]!.message;
                const showRoundDivider = shouldShowRoundDivider(
                  message,
                  previous,
                );
                const round = message.collaboration?.round ?? null;
                return (
                  <Fragment key={message.id}>
                    {showRoundDivider && round !== null && (
                      <div className="round-divider" role="separator">
                        <span>第 {round} 轮</span>
                      </div>
                    )}
                    <MessageCard
                      message={message}
                      name={name}
                      isContinuation={isContinuation && !showRoundDivider}
                      isModerator={
                        builtinModerator !== null &&
                        message.senderAgentId === builtinModerator.id
                      }
                      moderatorId={builtinModerator?.id ?? null}
                      agentNameById={agentNameById}
                      quoted={quoted}
                      quotedName={quoted === undefined ? null : senderName(quoted)}
                      replyToId={replyToId}
                      mentionNames={mentionNames}
                      durationMs={
                        runDurations[runIdByMessageId[message.id] ?? -1] ?? null
                      }
                      runId={runIdByMessageId[message.id] ?? null}
                      onInspectRun={openRunPage}
                      onReply={startReply}
                      onCopy={copyMessage}
                      onToggleReaction={toggleReaction}
                      onJumpToMessage={jumpToMessage}
                      copied={copiedMessageId === message.id}
                      reactionBusy={reactionBusyId === message.id}
                      flash={flashMessageId === message.id}
                      onLayoutChange={handleMessageLayoutChange}
                    />
                  </Fragment>
                );
              })
            )}
          </div>

          {!stickToBottom && (
            <button
              type="button"
              className="jump-latest"
              onClick={jumpToLatest}
              aria-label={
                unseenCount > 0
                  ? `回到最新，${unseenCount} 条未读`
                  : "回到最新"
              }
              title="回到最新"
            >
              <ChevronDownIcon />
              {unseenCount > 0 && (
                <span className="jump-latest-count">
                  {unseenCount > 99 ? "99+" : unseenCount}
                </span>
              )}
            </button>
          )}

          {composerError !== null && (
            <p className="banner error" role="alert">
              {composerError}
            </p>
          )}

          {messageNotice !== null && (
            <p className="banner error" role="alert">
              {messageNotice}
            </p>
          )}

          <form
            className="composer"
            onSubmit={send}
            onDragOver={(event) => event.preventDefault()}
            onDrop={handleComposerDrop}
          >
            <input
              ref={attachmentInputRef}
              type="file"
              multiple
              hidden
              aria-label="选择附件"
              onChange={(event) => {
                if (event.target.files !== null) {
                  addAttachments(event.target.files);
                }
                event.target.value = "";
              }}
            />
            {pendingAttachments.length > 0 && (
              <div className="composer-attachments" aria-label="待发送附件">
                {pendingAttachments.map((attachment) => (
                  <div key={attachment.key} className="attachment-chip">
                    {attachment.previewUrl !== null ? (
                      <img
                        className="attachment-chip-thumb"
                        src={attachment.previewUrl}
                        alt=""
                      />
                    ) : (
                      <span className="attachment-chip-icon" aria-hidden="true">
                        文件
                      </span>
                    )}
                    <span
                      className="attachment-chip-name"
                      title={attachment.file.name}
                    >
                      {attachment.file.name}
                    </span>
                    <span className="attachment-chip-size">
                      {formatFileSize(attachment.file.size)}
                    </span>
                    <button
                      type="button"
                      className="attachment-chip-remove"
                      aria-label={`移除附件 ${attachment.file.name}`}
                      onClick={() => removePendingAttachment(attachment.key)}
                      disabled={sending}
                    >
                      ×
                    </button>
                  </div>
                ))}
              </div>
            )}
            {replyTo !== null && (
              <div className="composer-reply" role="status">
                <span className="composer-reply-text">
                  回复 {senderName(replyTo)}：{summarizeReply(replyTo.content)}
                </span>
                <button
                  type="button"
                  className="composer-reply-cancel"
                  aria-label="取消回复"
                  onClick={() => setReplyTo(null)}
                  disabled={sending}
                >
                  ×
                </button>
              </div>
            )}
            <div className="composer-input">
              <button
                type="button"
                className="composer-attach"
                title="添加附件"
                aria-label="添加附件"
                onClick={() => attachmentInputRef.current?.click()}
                disabled={sending || discussionRunning}
              >
                <PaperclipIcon />
              </button>
              <textarea
                ref={textareaRef}
                value={draft}
                onChange={handleComposerChange}
                onKeyDown={handleComposerKeyDown}
                onPaste={handleComposerPaste}
                onSelect={(event) =>
                  setCursor(event.currentTarget.selectionStart)
                }
                onClick={(event) =>
                  setCursor(event.currentTarget.selectionStart)
                }
                placeholder={
                  discussionRunning
                    ? "讨论进行中，请等待结束或先停止讨论"
                    : room?.replyMode === "moderator"
                      ? "直接发送，内置主持人会接管；输入 @ 指定成员，输入 / 打开主持人命令"
                      : "输入消息并用 @ 指定成员；输入 / 打开主持人命令"
                }
                rows={1}
                disabled={sending || discussionRunning}
                aria-autocomplete="list"
                aria-expanded={completionOpen}
                aria-controls={
                  completionOpen ? "composer-completion" : undefined
                }
              />
              {completionOpen && (
                <ul
                  id="composer-completion"
                  className="completion-menu"
                  role="listbox"
                  aria-label={
                    trigger?.kind === "mention" ? "房间成员" : "主持人命令"
                  }
                >
                  {completionOptions.map((option, index) => (
                    <li key={option.key} role="presentation">
                      <button
                        type="button"
                        role="option"
                        aria-selected={index === activeCompletionIndex}
                        className={
                          index === activeCompletionIndex
                            ? "completion-option is-active"
                            : "completion-option"
                        }
                        onMouseDown={(event) => event.preventDefault()}
                        onClick={() => applyCompletion(option)}
                      >
                        <span className="completion-label">{option.label}</span>
                        <span className="completion-desc">
                          {option.description}
                        </span>
                      </button>
                    </li>
                  ))}
                </ul>
              )}
              <button
                type="submit"
                className="composer-send"
                title={uploading ? "上传中…" : sending ? "发送中…" : "发送"}
                aria-label={uploading ? "上传中" : sending ? "发送中" : "发送"}
                disabled={
                  sending ||
                  discussionRunning ||
                  (draft.trim() === "" && pendingAttachments.length === 0)
                }
              >
                <ArrowUpIcon />
              </button>
            </div>
            <div className="composer-actions">
              <span className="composer-hint">
                Enter 发送、Shift+Enter 换行；输入 @ 选择成员、/ 打开主持人命令；
                补全时 ↑↓ 选择、Enter 确认、Esc 关闭。支持拖放或粘贴图片。
                {room?.replyMode !== "moderator" && builtinModerator !== null
                  ? `需要讨论时请 @${builtinModerator.name} 说明主题。`
                  : ""}
              </span>
            </div>
          </form>
        </div>
      </div>

        </div>

        <aside id="room-side" className="room-side" aria-label="房间侧栏">
          <div className="room-side-top">
            <span>房间面板</span>
            <button
              ref={sideCloseRef}
              type="button"
              className="btn btn-ghost btn-sm"
              onClick={() => setSidebarOpen(false)}
            >
              关闭
            </button>
          </div>
          <div className="room-side-tabs" role="tablist" aria-label="房间侧栏">
            {ROOM_SIDE_TABS.map((tab) => {
              const count =
                tab.id === "members"
                  ? members.length
                  : tab.id === "queue"
                    ? (queue?.jobs.length ?? 0)
                    : 0;
              return (
                <button
                  key={tab.id}
                  id={`room-side-tab-${tab.id}`}
                  type="button"
                  role="tab"
                  className="room-side-tab"
                  aria-selected={sideTab === tab.id}
                  aria-controls={`room-side-panel-${tab.id}`}
                  tabIndex={sideTab === tab.id ? 0 : -1}
                  onClick={() => setSideTab(tab.id)}
                  onKeyDown={onSideTabKeyDown}
                >
                  {tab.label}
                  {count > 0 && (
                    <span className="room-side-tab-count">{count}</span>
                  )}
                </button>
              );
            })}
          </div>

          {discussion !== null && (
            <section
              className={`room-side-section discussion-banner is-${discussion.status}`}
              aria-label="房间讨论状态"
            >
              <div className="discussion-main">
                <p className="discussion-topic">{discussion.topic}</p>
                <div className="discussion-meta">
                  <span
                    className={`status-pill discussion-status ${discussion.status}`}
                  >
                    {discussionStatusLabel(discussion.status)}
                  </span>
                  <span>
                    {formatDiscussionRound(
                      discussion.currentRound,
                      discussion.maxRounds,
                    )}
                  </span>
                  <span>参与者：{discussionParticipantNames}</span>
                  {discussionStopReason !== null && (
                    <span>结束原因：{discussionStopReason}</span>
                  )}
                </div>
                {discussion.summary !== null &&
                  discussion.summary.trim() !== "" && (
                    <p
                      className="discussion-summary"
                      title={discussion.summary}
                    >
                      {plainDiscussionSummary(discussion.summary)}
                    </p>
                  )}
              </div>
              {(discussionRunning ||
                isResumableDiscussion(discussion.status)) && (
                <div className="discussion-actions">
                  {discussionRunning && (
                    <button
                      type="button"
                      className="btn btn-secondary btn-sm"
                      onClick={() => void stopDiscussion()}
                      disabled={stoppingDiscussion}
                    >
                      {stoppingDiscussion ? "停止中…" : "停止讨论"}
                    </button>
                  )}
                  {isResumableDiscussion(discussion.status) && (
                    <button
                      type="button"
                      className="btn btn-secondary btn-sm"
                      onClick={() => void resumeDiscussion()}
                      disabled={resumingDiscussion}
                    >
                      {resumingDiscussion ? "继续中…" : "继续讨论"}
                    </button>
                  )}
                </div>
              )}
            </section>
          )}

          <div
            id="room-side-panel-overview"
            role="tabpanel"
            aria-labelledby="room-side-tab-overview"
            className="room-side-tabpanel"
            hidden={sideTab !== "overview"}
          >
          <section className="room-side-section">
            <header className="room-side-head">
              <h2>房间状态</h2>
              <span className={connected ? "live on" : "live off"}>
                {connected ? "实时已连接" : "重连中…"}
              </span>
            </header>
            <dl className="room-side-facts">
              {builtinModerator !== null && (
                <div>
                  <dt>主持人</dt>
                  <dd>{builtinModerator.name}</dd>
                </div>
              )}
              <div>
                <dt>回复模式</dt>
                <dd>
                  {room?.replyMode === "moderator"
                    ? "主持人自动接管"
                    : "仅 @ 成员回复"}
                </dd>
              </div>
              {room !== null && (
                <div>
                  <dt>讨论上限</dt>
                  <dd>{room.discussionMaxRounds} 轮</dd>
                </div>
              )}
              {room !== null && (
                <div>
                  <dt>权限模式</dt>
                  <dd>
                    <select
                      className="room-side-select"
                      aria-label="房间权限模式"
                      value={room.permissionMode}
                      disabled={permissionSaving}
                      onChange={(event) => {
                        void changePermissionMode(
                          event.target.value as Room["permissionMode"],
                        );
                      }}
                    >
                      {(["plan", "auto", "full"] as const).map((mode) => (
                        <option key={mode} value={mode}>
                          {PERMISSION_MODE_LABELS[mode]}
                        </option>
                      ))}
                    </select>
                    <span className="room-side-note">
                      {permissionSaving
                        ? "保存中…"
                        : PERMISSION_MODE_HINTS[room.permissionMode]}
                    </span>
                  </dd>
                </div>
              )}
              {room !== null && (
                <div>
                  <dt>工作目录</dt>
                  <dd>
                    <span className="room-side-path" title={room.workspaceDirs.join("\n")}>
                      {room.workspaceDir}
                    </span>
                    {room.workspaceDirs.length > 1 && (
                      <span className="room-side-note">
                        另有 {room.workspaceDirs.length - 1} 个附加目录
                      </span>
                    )}
                  </dd>
                </div>
              )}
              <div>
                <dt>成员</dt>
                <dd>{members.length} 名</dd>
              </div>
            </dl>
          </section>

          {contextAgentId === null ? (
            <section className="room-side-section">
              <header className="room-side-head">
                <h2>上下文</h2>
              </header>
              <p className="muted small">添加成员后显示</p>
            </section>
          ) : contextPreview === null ? (
            contextLoading ? (
              <section className="room-side-section">
                <header className="room-side-head">
                  <h2>上下文</h2>
                  <span className="room-side-note">估算</span>
                </header>
                <p className="muted small">正在估算…</p>
              </section>
            ) : null
          ) : (
            <section className="room-side-section">
              <header className="room-side-head">
                <h2>上下文</h2>
                <span className="room-side-note">估算</span>
              </header>
              <div className="context-card">
                <p className="context-metric">
                  {formatContextMetric(contextPreview)}
                </p>
                <p className="context-summary">
                  {formatContextSummary(contextPreview)}
                </p>
                <p className="context-budget">
                  预算 {contextPreview.budgetChars} 字
                  {contextPreview.enabled ? "" : " · 压缩已关闭"}
                </p>
                <Link className="context-settings" to="/settings">
                  调整预算
                </Link>
              </div>
            </section>
          )}

          {(runningRuns.length > 0 || collaboration !== null) && (
            <section className="room-side-section">
              <header className="room-side-head">
                <h2>实时状态</h2>
              </header>
              <div className="room-live" aria-label="房间实时状态">
                {runningRuns.length > 0 && (
                  <section className="live-card" aria-label="正在运行的成员">
                    <header className="live-card-head">
                      <span className="live-dot" aria-hidden="true" />
                      <strong className="live-card-title">正在运行</strong>
                      <span className="live-card-count">
                        {runningRuns.length}
                      </span>
                    </header>
                    <div className="live-run-list">
                      {runningRuns.map((run) => (
                        <button
                          key={run.messageId}
                          type="button"
                          className="live-run-item"
                          title="查看这次运行的实时细节"
                          onClick={() => openRunPage(run.runId)}
                        >
                          <span className="live-run-name">{run.name}</span>
                          <span className="live-run-time">
                            {formatElapsed(run.elapsedMs)}
                          </span>
                        </button>
                      ))}
                    </div>
                  </section>
                )}

                {collaboration !== null && (
                  <section
                    className={`live-card live-collab is-${collaboration.status}`}
                    aria-label="房间委派批次状态"
                  >
                    <header className="live-card-head">
                      <strong className="live-card-title">委派批次</strong>
                      <span
                        className={`status-pill collaboration-status ${collaboration.status}`}
                      >
                        {collaborationStatusLabel(collaboration.status)}
                      </span>
                      <span className="live-meta">
                        {formatCollaborationRound(
                          collaboration.currentRound,
                          collaboration.maxRounds,
                        )}
                      </span>
                      <span className="live-counts" aria-label="本轮指派统计">
                        {collaborationCounts.running > 0 && (
                          <span className="live-count is-running">
                            {collaborationCounts.running} 进行中
                          </span>
                        )}
                        {collaborationCounts.queued > 0 && (
                          <span className="live-count is-queued">
                            {collaborationCounts.queued} 排队中
                          </span>
                        )}
                        {collaborationCounts.succeeded > 0 && (
                          <span className="live-count is-succeeded">
                            {collaborationCounts.succeeded} 成功
                          </span>
                        )}
                        {collaborationCounts.failed > 0 && (
                          <span className="live-count is-failed">
                            {collaborationCounts.failed} 失败
                          </span>
                        )}
                        {collaborationCounts.total === 0 && (
                          <span className="live-count">暂无指派</span>
                        )}
                      </span>
                      {collaborationRunning && collaboration.stopRequested && (
                        <span className="live-note">
                          已请求停止，本轮结束后生效
                        </span>
                      )}
                      {collaborationStopReason !== null && (
                        <span className="live-note">
                          结束原因：{collaborationStopReason}
                        </span>
                      )}
                      {collaborationRunning && (
                        <button
                          type="button"
                          className="btn btn-secondary btn-sm live-action"
                          onClick={() => void stopCollaboration()}
                          disabled={
                            stoppingCollaboration ||
                            collaboration.stopRequested
                          }
                        >
                          {stoppingCollaboration
                            ? "停止中…"
                            : collaboration.stopRequested
                              ? "等待本轮结束"
                              : "停止委派"}
                        </button>
                      )}
                    </header>
                    {collaborationRoundAssignments.length > 0 && (
                      <div className="live-members" aria-label="本轮成员状态">
                        {collaborationRoundAssignments.map((assignment) => (
                          <span
                            key={assignment.id}
                            className={`collaboration-member is-${assignment.status}`}
                          >
                            <span className="collaboration-member-name">
                              {agentNameById.get(assignment.agentId) ??
                                `智能体#${assignment.agentId}`}
                            </span>
                            <span className="collaboration-member-status">
                              {assignmentStatusLabel(assignment.status)}
                            </span>
                          </span>
                        ))}
                      </div>
                    )}
                    {collaborationSummary !== "" && (
                      <p className="live-summary">{collaborationSummary}</p>
                    )}
                  </section>
                )}
              </div>
            </section>
          )}

          </div>

          <section
            id="room-side-panel-queue"
            role="tabpanel"
            aria-labelledby="room-side-tab-queue"
            className="room-side-section"
            aria-label="房间队列"
            hidden={sideTab !== "queue"}
          >
            <header className="room-side-head">
              <h2>队列</h2>
              <div className="queue-head-actions">
                <span className="room-side-note">
                  {queue === null
                    ? "读取中…"
                    : queueHeadline(queue.jobs, queue.paused)}
                </span>
                <button
                  type="button"
                  className="btn btn-ghost btn-sm queue-pause"
                  onClick={() => void toggleQueuePaused()}
                  disabled={queue === null || queuePauseBusy}
                >
                  {queuePauseBusy
                    ? "处理中…"
                    : queue?.paused
                      ? "恢复队列"
                      : "暂停队列"}
                </button>
              </div>
            </header>
            {queueNotice !== null && (
              <p className="queue-notice" role="alert">
                {queueNotice}
              </p>
            )}
            {queueError && (
              <p className="queue-note">
                {queue === null ? "队列刷新失败" : "队列刷新失败，显示上次结果"}
              </p>
            )}
            {queue?.paused && (
              <p className="queue-paused">
                队列已暂停，恢复后继续执行；运行中的任务不受影响。
              </p>
            )}
            {queue === null ? (
              <p className="muted small">正在读取队列…</p>
            ) : queue.jobs.length === 0 ? (
              <p className="muted small">
                {queue.paused ? "队列已暂停，暂无任务" : "队列空闲"}
              </p>
            ) : (
              <ul className="queue-list">
                {sortQueueJobs(queue.jobs).map((job) => (
                  <li key={job.id} className={`queue-item is-${job.state}`}>
                    <div className="queue-item-main">
                      <span className={`queue-state is-${job.state}`}>
                        {queueStateLabel(job.state)}
                      </span>
                      <span className="queue-agent" title={job.label}>
                        {job.label}
                      </span>
                      <span className="queue-kind">
                        {queueKindLabel(job.kind)}
                      </span>
                    </div>
                    <div className="queue-item-meta">
                      <span>
                        {job.state === "running"
                          ? `已运行 ${formatWait(now - (job.startedAt ?? job.enqueuedAt))}`
                          : `#${job.position} · 等待 ${formatWait(now - job.enqueuedAt)}`}
                      </span>
                      {canCancelQueueJob(job) && (
                        <button
                          type="button"
                          className="btn btn-ghost btn-sm queue-cancel"
                          onClick={() => void cancelQueueJob(job.id)}
                          disabled={cancellingQueueJobId !== null}
                        >
                          {cancellingQueueJobId === job.id
                            ? "取消中…"
                            : "取消"}
                        </button>
                      )}
                    </div>
                  </li>
                ))}
              </ul>
            )}
          </section>

          <section
            id="room-side-panel-files"
            role="tabpanel"
            aria-labelledby="room-side-tab-files"
            className="room-side-section"
            aria-label="房间文件"
            hidden={sideTab !== "files"}
          >
            <header className="room-side-head">
              <h2>文件</h2>
              <button
                type="button"
                className="btn btn-ghost btn-sm"
                onClick={() => setFilesRefreshKey((key) => key + 1)}
              >
                刷新
              </button>
            </header>
            {filesError !== null && (
              <p className="queue-note" role="alert">
                {filesError}
              </p>
            )}
            {files === null ? (
              <p className="muted small">正在读取文件…</p>
            ) : files.roots.length === 0 ? (
              <p className="muted small">工作区暂无文件</p>
            ) : (
              <div className="files-roots">
                {files.roots.map((root) => (
                  <section
                    key={root.id}
                    className="files-root"
                    aria-label={root.label}
                  >
                    <div className="files-root-head">
                      <span className="files-root-label">{root.label}</span>
                      <span className="files-root-count">{root.files.length}</span>
                    </div>
                    <p className="files-root-path" title={root.path}>
                      {root.path}
                    </p>
                    {root.files.length === 0 ? (
                      <p className="muted small">暂无文件</p>
                    ) : (
                      <ul className="files-list">
                        {root.files.map((file) => (
                          <li key={file.path} className="file-row">
                            <a
                              className="file-link"
                              href={fileDownloadUrl(roomId, root.id, file.path)}
                              download
                              title={file.path}
                            >
                              {file.path}
                            </a>
                            <span className="file-meta">
                              {formatFileSize(file.size)}
                            </span>
                          </li>
                        ))}
                      </ul>
                    )}
                    {root.truncated && (
                      <p className="files-truncated">仅显示前 200 个文件</p>
                    )}
                  </section>
                ))}
              </div>
            )}
          </section>

          {members.length > 0 && (
            <section
              id="room-side-panel-members"
              role="tabpanel"
              aria-labelledby="room-side-tab-members"
              className="room-side-section"
              hidden={sideTab !== "members"}
            >
              <header className="room-side-head">
                <h2>成员</h2>
                <span className="muted small">{members.length} 名</span>
              </header>
              <div className="room-side-members">
                {members.map((member) => (
                  <button
                    key={member.id}
                    type="button"
                    className={
                      member.builtinRole === "moderator"
                        ? "mention-chip is-moderator"
                        : "mention-chip"
                    }
                    onClick={() => insertMention(member.name)}
                    title={`插入 @${member.name}`}
                  >
                    <AgentAvatar
                      id={member.id}
                      name={member.name}
                      size="sm"
                    />
                    @{member.name}
                    {member.builtinRole === "moderator" && (
                      <span className="badge badge-moderator">主持人</span>
                    )}
                  </button>
                ))}
              </div>
            </section>
          )}
        </aside>
      </div>

      {sidebarOpen && (
        <button
          type="button"
          className="room-side-backdrop"
          aria-label="关闭房间侧栏"
          onClick={() => setSidebarOpen(false)}
        />
      )}

      {cleanupResult !== null && (
        <p className="banner success" role="status">
          {cleanupResult}
        </p>
      )}

      {sessionPanelOpen && (
        <SessionPanel
          roomId={roomId}
          members={members}
          onClose={() => setSessionPanelOpen(false)}
        />
      )}

      {cleanupOpen && (
        <div className="modal-layer" role="presentation">
          <div
            className="modal-card clean-modal"
            role="dialog"
            aria-modal="true"
            aria-labelledby="cleanup-title"
          >
            <header className="modal-head">
              <h2 id="cleanup-title">清理历史消息</h2>
              <button
                type="button"
                className="btn btn-ghost btn-sm"
                onClick={closeCleanup}
                disabled={cleaning}
              >
                关闭
              </button>
            </header>
            <div className="clean-body">
              <label className="field">
                <span className="field-label">保留最近 N 条纯聊天消息</span>
                <input
                  ref={cleanupInputRef}
                  type="number"
                  min={0}
                  max={5000}
                  step={1}
                  value={keepRecentInput}
                  onChange={(event) => {
                    setKeepRecentInput(event.target.value);
                    setCleanupError(null);
                  }}
                  disabled={cleaning}
                />
              </label>
              <p className="clean-hint">
                0 表示清理全部聊天窗口历史。纯聊天消息会被删除；讨论、任务、委派记录本身保留，它们的旧消息会从聊天窗口隐藏。房间有运行中的任务、讨论或委派时无法清理。
              </p>
              <p className="clean-preview" aria-live="polite">
                预计清理 <strong>{preview.deleting}</strong> 条，保留{" "}
                <strong>{preview.keeping}</strong> 条（共 {preview.eligible}{" "}
                条纯聊天消息）
                {preview.hidden > 0
                  ? `；另将隐藏 ${preview.hidden} 条讨论/任务/委派记录`
                  : ""}
              </p>
              {cleanupError !== null && (
                <p className="banner error" role="alert">
                  {cleanupError}
                </p>
              )}
            </div>
            <footer className="modal-actions">
              <button
                type="button"
                className="btn btn-secondary"
                onClick={closeCleanup}
                disabled={cleaning}
              >
                取消
              </button>
              <button
                type="button"
                className="btn btn-danger"
                onClick={() => void submitCleanup()}
                disabled={
                  cleaning ||
                  (preview.deleting === 0 && preview.hidden === 0)
                }
              >
                {cleaning ? "清理中…" : "确认清理"}
              </button>
            </footer>
          </div>
        </div>
      )}
    </section>
  );
}
