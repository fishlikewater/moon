import {
  lazy,
  memo,
  Suspense,
  useCallback,
  useEffect,
  useRef,
  useState,
  type ChangeEvent,
  type ClipboardEvent,
  type DragEvent,
  type FormEvent,
  type KeyboardEvent,
} from "react";
import { Link, useNavigate, useParams } from "react-router-dom";
import type { Agent, Message, MessageAttachment, Room } from "@moon/shared";
import { api, errorMessage } from "../api/client";
import { AgentAvatar } from "../components/AgentAvatar";
import {
  DEFAULT_ATTACHMENT_LIMITS,
  filterPendingFiles,
  formatFileSize,
  isImageAttachment,
} from "../components/attachmentView";
import { composerKeyAction } from "../components/composerKeys";
import { ContextMenu, useContextMenu } from "../components/ContextMenu";
import {
  buildMessageLink,
  buildMessageMenuItems,
} from "../components/messageContextMenu";
import { useRoomChrome } from "../components/layout/roomChrome";
import SessionPanel from "../components/SessionPanel";
import { useRoomChat } from "../hooks/useRoomChat";
import { changeDirectPermissionMode } from "./directChatPermission";
import { uploadAndSendDirectMessage } from "./directChatSend";

// Markdown 依赖体积较大；与房间页一致按需加载。
const MarkdownContent = lazy(() => import("../components/MarkdownContent"));

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

interface PendingAttachment {
  key: string;
  file: File;
  previewUrl: string | null;
}

function parseRoomId(raw: string | undefined): number | null {
  if (raw === undefined || !/^\d+$/.test(raw)) {
    return null;
  }
  const value = Number(raw);
  return Number.isSafeInteger(value) && value > 0 ? value : null;
}

function formatTime(ms: number): string {
  return new Date(ms).toLocaleTimeString("zh-CN", {
    hour12: false,
    hour: "2-digit",
    minute: "2-digit",
  });
}

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

/** 清屏只删除消息，与「删除对话」区分，避免两个垃圾桶图标。 */
function ClearIcon() {
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
      <path d="M4 20h16" />
      <path d="M14.5 4.5l5 5-8 8H6l-1.5-1.5a2 2 0 0 1 0-2.8l7-7a2 2 0 0 1 3 0Z" />
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

/** 消息附件：图片给缩略预览，其余给下载链接。 */
function MessageAttachments({
  attachments,
}: {
  attachments: readonly MessageAttachment[];
}) {
  if (attachments.length === 0) {
    return null;
  }
  return (
    <div className="msg-attachments" aria-label="附件">
      {attachments.map((attachment) => {
        const href = api.attachments.url(attachment.id);
        if (isImageAttachment(attachment.mimeType)) {
          return (
            <a
              key={attachment.id}
              className="msg-attachment-image"
              href={href}
              target="_blank"
              rel="noreferrer"
              title={`${attachment.name}（${formatFileSize(attachment.size)}）`}
            >
              <img src={href} alt={attachment.name} loading="lazy" />
            </a>
          );
        }
        return (
          <a
            key={attachment.id}
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
      })}
    </div>
  );
}

/** 发送按钮可用性：非发送中且有正文或附件。 */
export function canSendDirectMessage(
  draft: string,
  attachmentCount: number,
  sending: boolean,
): boolean {
  return !sending && (draft.trim() !== "" || attachmentCount > 0);
}

interface DirectMessageProps {
  message: Message;
  agent: Agent;
  runId: number | null;
  onInspectRun: (runId: number) => void;
  onLayoutChange: () => void;
}

/** 直聊消息：复用房间消息的视觉类名，但不渲染讨论/委派/反应等房间元素。 */
export const DirectMessage = memo(function DirectMessage({
  message,
  agent,
  runId,
  onInspectRun,
  onLayoutChange,
}: DirectMessageProps) {
  const isUser = message.senderType === "user";
  const isProgress = message.kind === "progress";
  const [copyFeedback, setCopyFeedback] = useState<{
    kind: "text" | "link";
    status: "done" | "failed";
  } | null>(null);
  const messageMenu = useContextMenu();

  useEffect(() => {
    if (copyFeedback === null) {
      return;
    }
    const timer = window.setTimeout(() => setCopyFeedback(null), 1500);
    return () => window.clearTimeout(timer);
  }, [copyFeedback]);

  const copyText = async (): Promise<void> => {
    try {
      await navigator.clipboard.writeText(message.content);
      setCopyFeedback({ kind: "text", status: "done" });
    } catch {
      setCopyFeedback({ kind: "text", status: "failed" });
    }
  };

  const copyLink = async (): Promise<void> => {
    try {
      await navigator.clipboard.writeText(
        buildMessageLink(
          window.location.origin,
          window.location.pathname,
          message.id,
        ),
      );
      setCopyFeedback({ kind: "link", status: "done" });
    } catch {
      setCopyFeedback({ kind: "link", status: "failed" });
    }
  };

  const actionLabel = (
    kind: "text" | "link",
    idle: string,
    done: string,
    failed: string,
  ): string => {
    if (copyFeedback?.kind !== kind) {
      return idle;
    }
    return copyFeedback.status === "done" ? done : failed;
  };

  const messageMenuItems = buildMessageMenuItems(
    { canReply: false, canReact: false, runId },
    {
      onReply: () => {},
      onCopy: () => void copyText(),
      onCopyLink: () => void copyLink(),
      onToggleReaction: () => {},
      onInspectRun: () => {
        if (runId !== null) {
          onInspectRun(runId);
        }
      },
    },
  );

  return (
    <article
      id={`message-${message.id}`}
      aria-haspopup="menu"
      className={`msg msg-${message.senderType} msg-${message.status}${
        isProgress ? " is-progress" : ""
      }`}
      onContextMenu={(event) => {
        messageMenu.openAt(event);
      }}
    >
      {isUser ? (
        <span className="avatar avatar-sm avatar-user" aria-hidden="true">
          我
        </span>
      ) : (
        <AgentAvatar
          id={message.senderAgentId ?? agent.id}
          name={agent.name}
          size="sm"
        />
      )}

      <div className="msg-body">
        <header className="msg-head">
          <strong>{isUser ? "我" : agent.name}</strong>
          <time>{formatTime(message.createdAt)}</time>
          {message.status === "streaming" && (
            <span className="direct-streaming">生成中…</span>
          )}
          {message.status === "failed" && (
            <span className="status-pill blocked">失败</span>
          )}
          <div className="msg-actions" role="group" aria-label="消息操作">
            <button
              type="button"
              className="msg-action"
              onClick={() => void copyText()}
            >
              {actionLabel("text", "复制", "已复制", "复制失败")}
            </button>
            <button
              type="button"
              className="msg-action"
              onClick={() => void copyLink()}
            >
              {actionLabel("link", "链接", "已复制链接", "链接失败")}
            </button>
          </div>
        </header>

        {isProgress ? (
          <p className="msg-text muted">{message.content}</p>
        ) : (
          <Suspense fallback={<p className="msg-text">{message.content}</p>}>
            <MarkdownContent text={message.content} onRendered={onLayoutChange} />
          </Suspense>
        )}

        <MessageAttachments attachments={message.attachments ?? []} />

        {runId !== null && (
          <div className="msg-run">
            <button
              type="button"
              className="btn btn-ghost btn-sm"
              onClick={() => onInspectRun(runId)}
            >
              运行详情
            </button>
          </div>
        )}
      </div>
      {messageMenu.open && (
        <ContextMenu
          open
          position={messageMenu.position}
          items={messageMenuItems}
          onClose={messageMenu.close}
          ariaLabel="消息操作"
        />
      )}
    </article>
  );
});

/** 空态引导：头像、人设与沙盒说明；无消息时渲染。 */
export function DirectEmptyState({ agent }: { agent: Agent }) {
  return (
    <div className="empty-state direct-empty">
      <AgentAvatar id={agent.id} name={agent.name} size="lg" />
      <p className="direct-empty-name">与 {agent.name} 对话</p>
      <p>{agent.persona}</p>
      <p className="muted">
        TA 在独立沙盒工作目录中工作，不影响智能体在其他房间的工作区。
        直接输入即可开始，无需 @ 成员。
      </p>
    </div>
  );
}

interface DirectChatSideFactsProps {
  agent: Agent;
  room: Room;
  connected: boolean;
  runningCount: number;
  permissionSaving: boolean;
  onChangePermission: (mode: Room["permissionMode"]) => void;
}

/** 右侧抽屉内容：连接、运行、harness/模型、沙盒与权限；动作入口在 rail。 */
export function DirectChatSideFacts({
  agent,
  room,
  connected,
  runningCount,
  permissionSaving,
  onChangePermission,
}: DirectChatSideFactsProps) {
  const model = agent.modelConfig?.model ?? "默认";
  const reasoning = agent.modelConfig?.reasoningLevel;
  return (
    <>
      <section className="room-side-section">
        <header className="room-side-head">
          <h2>对话状态</h2>
          <span className={connected ? "live on" : "live off"}>
            {connected ? "实时已连接" : "重连中…"}
          </span>
        </header>
        <dl className="room-side-facts">
          <div>
            <dt>运行中</dt>
            <dd>{runningCount > 0 ? `${runningCount} 个` : "空闲"}</dd>
          </div>
          <div>
            <dt>Harness</dt>
            <dd>
              <span className="badge badge-harness">{agent.harness}</span>
            </dd>
          </div>
          <div>
            <dt>模型</dt>
            <dd>
              <span className="chip chip-mono">
                {model}
                {typeof reasoning === "string" && reasoning.trim() !== ""
                  ? ` · ${reasoning}`
                  : ""}
              </span>
            </dd>
          </div>
          <div>
            <dt>沙盒</dt>
            <dd>
              <span className="room-side-path" title={room.workspaceDir}>
                {room.workspaceDir}
              </span>
            </dd>
          </div>
        </dl>
      </section>

      <section className="room-side-section">
        <header className="room-side-head">
          <h2>权限</h2>
        </header>
        <dl className="room-side-facts">
          <div>
            <dt>权限模式</dt>
            <dd>
              <select
                className="room-side-select"
                aria-label="直聊权限模式"
                value={room.permissionMode}
                disabled={permissionSaving}
                onChange={(event) => {
                  onChangePermission(
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
        </dl>
      </section>
    </>
  );
}

/**
 * 删除对话确认弹层：运行中禁用确认，删除进行中与失败原因都留在弹层内。
 *
 * 只负责展示；API 调用与跳转由页面持有。
 */
export function DirectDeleteDialog({
  agentName,
  runningCount,
  deleting,
  error,
  onCancel,
  onConfirm,
}: {
  agentName: string;
  runningCount: number;
  deleting: boolean;
  error: string | null;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const blocked = runningCount > 0;
  return (
    <div className="modal-layer">
      <div
        className="modal-card"
        role="dialog"
        aria-modal="true"
        aria-labelledby="direct-delete-title"
      >
        <header className="modal-head">
          <h2 id="direct-delete-title" className="card-title">
            删除对话
          </h2>
          <button
            type="button"
            className="btn btn-ghost btn-sm"
            aria-label="关闭删除对话框"
            onClick={onCancel}
            disabled={deleting}
          >
            关闭
          </button>
        </header>
        <p className="muted">
          删除与 {agentName}{" "}
          的对话后，消息、运行记录与会话都会删除，无法恢复；工作区文件保留。
        </p>
        {blocked && (
          <p className="field-hint">有运行中的回复，等待完成或停止后再删除。</p>
        )}
        {error !== null && (
          <p className="banner error" role="alert">
            {error}
          </p>
        )}
        <div className="modal-actions">
          <button
            type="button"
            className="btn btn-secondary"
            onClick={onCancel}
            disabled={deleting}
          >
            取消
          </button>
          <button
            type="button"
            className="btn btn-danger"
            onClick={onConfirm}
            disabled={deleting || blocked}
          >
            {deleting ? "删除中…" : "确认删除"}
          </button>
        </div>
      </div>
    </div>
  );
}

/**
 * 智能体直聊页：`/chat/room/:roomId`。
 *
 * 直聊是隐藏的单成员房间，消息、流式、会话、附件与运行详情都复用房间链路；
 * 页面只保留 1:1 需要的展示与操作，不渲染成员/讨论/委派入口。
 */
export default function DirectChatPage() {
  const { roomId: rawRoomId } = useParams<{ roomId: string }>();
  const roomId = parseRoomId(rawRoomId);
  const navigate = useNavigate();
  const { setChrome } = useRoomChrome();

  const [room, setRoom] = useState<Room | null>(null);
  const [agent, setAgent] = useState<Agent | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [draft, setDraft] = useState("");
  const [sending, setSending] = useState(false);
  const [uploading, setUploading] = useState(false);
  const [composerError, setComposerError] = useState<string | null>(null);
  const [pendingAttachments, setPendingAttachments] = useState<
    PendingAttachment[]
  >([]);
  const [permissionSaving, setPermissionSaving] = useState(false);
  const [sessionPanelOpen, setSessionPanelOpen] = useState(false);
  const [clearOpen, setClearOpen] = useState(false);
  const [clearing, setClearing] = useState(false);
  const [clearError, setClearError] = useState<string | null>(null);
  const [clearNotice, setClearNotice] = useState<string | null>(null);
  const [directSideOpen, setDirectSideOpen] = useState(false);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [deleteError, setDeleteError] = useState<string | null>(null);

  // 清屏统计只做短暂反馈；再次清屏会写入新提示并重置计时。
  useEffect(() => {
    if (clearNotice === null) {
      return;
    }
    const timer = window.setTimeout(() => setClearNotice(null), 4000);
    return () => window.clearTimeout(timer);
  }, [clearNotice]);

  const attachmentInputRef = useRef<HTMLInputElement | null>(null);
  const attachmentKeyRef = useRef(0);
  const sideToggleRef = useRef<HTMLButtonElement | null>(null);
  const sideCloseRef = useRef<HTMLButtonElement | null>(null);
  const sidebarWasOpenRef = useRef(false);
  const pendingAttachmentsRef = useRef<PendingAttachment[]>([]);

  const chat = useRoomChat(roomId, { onError: setError });
  const {
    messages,
    runningCount,
    announcement,
    unseenCount,
    stickToBottom,
    historyLoading,
    connected,
    listRef,
    onListScroll,
    onMessageLayout,
    jumpToLatest,
    mergeMessage,
    reloadMessages,
  } = chat;

  useEffect(() => {
    pendingAttachmentsRef.current = pendingAttachments;
  }, [pendingAttachments]);

  // 离开页面时释放仍未发送的图片预览 URL。
  useEffect(() => {
    return () => {
      for (const attachment of pendingAttachmentsRef.current) {
        if (attachment.previewUrl !== null) {
          URL.revokeObjectURL(attachment.previewUrl);
        }
      }
    };
  }, []);

  // 直聊与智能体一一对应：从直聊列表里按 id 找到房间，再解析唯一成员。
  useEffect(() => {
    if (roomId === null) {
      setLoading(false);
      setError("非法的直聊地址");
      return;
    }

    let cancelled = false;
    setLoading(true);
    setError(null);
    setRoom(null);
    setAgent(null);
    void (async () => {
      try {
        const [directRooms, agents] = await Promise.all([
          api.rooms.list("direct"),
          api.agents.list(),
        ]);
        if (cancelled) {
          return;
        }
        const found = directRooms.find((candidate) => candidate.id === roomId);
        if (found === undefined) {
          setError("直聊不存在或已被删除");
          return;
        }
        const memberId = found.memberIds[0];
        const member =
          memberId === undefined
            ? null
            : (agents.find((candidate) => candidate.id === memberId) ?? null);
        if (member === null) {
          setError("直聊成员不存在");
          return;
        }
        setRoom(found);
        setAgent(member);
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

  // 顶栏只显示智能体身份；状态与操作收进右侧图标栏和抽屉。
  useEffect(() => {
    setChrome(
      agent === null
        ? null
        : {
            title: `与 ${agent.name} 对话`,
            backTo: "/agents",
            backLabel: "返回智能体",
          },
    );
    return () => setChrome(null);
  }, [agent, setChrome]);

  // 侧栏默认收起；打开时焦点进入浮层，关闭时还给 rail 开关，Esc 关闭。
  useEffect(() => {
    if (directSideOpen) {
      sidebarWasOpenRef.current = true;
      sideCloseRef.current?.focus();
      const closeOnEscape = (event: globalThis.KeyboardEvent) => {
        if (event.key === "Escape") {
          setDirectSideOpen(false);
        }
      };
      window.addEventListener("keydown", closeOnEscape);
      return () => window.removeEventListener("keydown", closeOnEscape);
    }
    if (sidebarWasOpenRef.current) {
      sidebarWasOpenRef.current = false;
      sideToggleRef.current?.focus();
    }
  }, [directSideOpen]);

  const confirmDelete = async () => {
    if (roomId === null || deleting || runningCount > 0) {
      return;
    }
    setDeleting(true);
    setDeleteError(null);
    try {
      await api.rooms.remove(roomId);
      navigate("/agents");
    } catch (cause) {
      setDeleteError(errorMessage(cause));
    } finally {
      setDeleting(false);
    }
  };

  const changePermissionMode = async (mode: Room["permissionMode"]) => {
    if (room === null || permissionSaving) {
      return;
    }
    setPermissionSaving(true);
    setError(null);
    try {
      const updated = await changeDirectPermissionMode(room, mode, {
        update: api.rooms.update,
      });
      if (updated !== null) {
        setRoom(updated);
      }
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setPermissionSaving(false);
    }
  };

  const addAttachments = useCallback((files: FileList | File[]) => {
    const { accepted, errors } = filterPendingFiles(
      pendingAttachmentsRef.current.length,
      Array.from(files),
      DEFAULT_ATTACHMENT_LIMITS,
    );
    if (errors.length > 0) {
      setComposerError(errors[0] ?? null);
    }
    if (accepted.length === 0) {
      return;
    }
    setPendingAttachments((previous) => [
      ...previous,
      ...accepted.map((file) => ({
        key: `direct-attach-${attachmentKeyRef.current++}`,
        file,
        previewUrl: isImageAttachment(file.type)
          ? URL.createObjectURL(file)
          : null,
      })),
    ]);
  }, []);

  const removePendingAttachment = useCallback((key: string) => {
    setPendingAttachments((previous) => {
      const target = previous.find((attachment) => attachment.key === key);
      if (target?.previewUrl !== null && target?.previewUrl !== undefined) {
        URL.revokeObjectURL(target.previewUrl);
      }
      return previous.filter((attachment) => attachment.key !== key);
    });
  }, []);

  const clearPendingAttachments = useCallback(() => {
    for (const attachment of pendingAttachmentsRef.current) {
      if (attachment.previewUrl !== null) {
        URL.revokeObjectURL(attachment.previewUrl);
      }
    }
    setPendingAttachments([]);
  }, []);

  const handleComposerPaste = (event: ClipboardEvent<HTMLTextAreaElement>) => {
    const files = event.clipboardData.files;
    if (files.length > 0) {
      event.preventDefault();
      addAttachments(files);
    }
  };

  const handleComposerDrop = (event: DragEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (event.dataTransfer.files.length > 0) {
      addAttachments(event.dataTransfer.files);
    }
  };

  const handleComposerChange = (event: ChangeEvent<HTMLTextAreaElement>) => {
    setDraft(event.target.value);
  };

  const handleComposerKeyDown = (
    event: KeyboardEvent<HTMLTextAreaElement>,
  ) => {
    const action = composerKeyAction({
      key: event.key,
      shiftKey: event.shiftKey,
      isComposing: event.nativeEvent.isComposing,
      completionOpen: false,
    });
    if (action === "send") {
      event.preventDefault();
      event.currentTarget.form?.requestSubmit();
    }
  };

  const send = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (roomId === null || sending) {
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
    try {
      const userMessage = await uploadAndSendDirectMessage(
        {
          upload: api.attachments.upload,
          send: api.messages.send,
          remove: api.attachments.remove,
        },
        roomId,
        content,
        attachments.map((attachment) => attachment.file),
      );
      mergeMessage(userMessage);
      setDraft("");
      clearPendingAttachments();
    } catch (cause) {
      setComposerError(errorMessage(cause));
    } finally {
      setUploading(false);
      setSending(false);
    }
  };

  const submitClear = async () => {
    if (roomId === null || clearing) {
      return;
    }
    setClearing(true);
    setClearError(null);
    try {
      const result = await api.messages.cleanup(roomId, 0);
      setClearOpen(false);
      setClearNotice(
        `已清屏：删除 ${result.deleted} 条，隐藏 ${result.hidden} 条`,
      );
      await reloadMessages();
    } catch (cause) {
      setClearError(errorMessage(cause));
    } finally {
      setClearing(false);
    }
  };

  if (loading || historyLoading) {
    return (
      <section className="page">
        <p className="muted">加载中…</p>
      </section>
    );
  }

  if (error !== null || roomId === null || room === null || agent === null) {
    return (
      <section className="page">
        <p className="banner error" role="alert">
          {error ?? "直聊不可用"}
        </p>
        <Link className="back-link" to="/agents">
          返回智能体
        </Link>
      </section>
    );
  }

  return (
    <section className="page direct-page page-fixed">
      <p className="sr-only" role="status" aria-live="polite">
        {announcement}
      </p>

      <div className={`direct-layout${directSideOpen ? " side-open" : ""}`}>
        <div className="direct-main">
          <div
            className="direct-rail"
            role="toolbar"
            aria-label="对话操作"
            aria-orientation="vertical"
          >
            <button
              type="button"
              className="direct-rail-button"
              title="运行记录"
              aria-label="运行记录"
              onClick={() => navigate(`/rooms/${roomId}/runs`)}
            >
              <HistoryIcon />
            </button>
            <button
              type="button"
              className="direct-rail-button"
              title="会话"
              aria-label="会话"
              onClick={() => setSessionPanelOpen(true)}
            >
              <SessionsIcon />
            </button>
            <button
              type="button"
              className="direct-rail-button is-danger"
              title="清屏"
              aria-label="清屏"
              onClick={() => {
                setClearError(null);
                setClearNotice(null);
                setClearOpen(true);
              }}
            >
              <ClearIcon />
            </button>
            <button
              type="button"
              className="direct-rail-button is-danger"
              title="删除对话"
              aria-label="删除对话"
              onClick={() => {
                setDeleteError(null);
                setDeleteOpen(true);
              }}
            >
              <TrashIcon />
            </button>
            <button
              ref={sideToggleRef}
              type="button"
              className="direct-rail-button"
              title={directSideOpen ? "收起侧栏" : "展开侧栏"}
              aria-label={directSideOpen ? "收起侧栏" : "展开侧栏"}
              aria-expanded={directSideOpen}
              aria-controls="direct-side"
              onClick={() => setDirectSideOpen((open) => !open)}
            >
              <PanelIcon />
            </button>
          </div>

      <div className="chat-shell">
        <div className="chat-panel">
          <div className="message-list" ref={listRef} onScroll={onListScroll}>
            {messages.length === 0 ? (
              <DirectEmptyState agent={agent} />
            ) : (
              messages.map((message) => (
                <DirectMessage
                  key={message.id}
                  message={message}
                  agent={agent}
                  runId={chat.runIdByMessageId[message.id] ?? null}
                  onInspectRun={(runId) =>
                    navigate(`/rooms/${roomId}/runs/${runId}`)
                  }
                  onLayoutChange={onMessageLayout}
                />
              ))
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
              ↓
              {unseenCount > 0 && (
                <span className="jump-latest-count">
                  {unseenCount > 99 ? "99+" : unseenCount}
                </span>
              )}
            </button>
          )}

          {clearNotice !== null && (
            <p className="banner" role="status">
              {clearNotice}
            </p>
          )}

          {composerError !== null && (
            <p className="banner error" role="alert">
              {composerError}
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
            <div className="composer-input">
              <button
                type="button"
                className="composer-attach"
                title="添加附件"
                aria-label="添加附件"
                onClick={() => attachmentInputRef.current?.click()}
                disabled={sending}
              >
                <PaperclipIcon />
              </button>
              <textarea
                value={draft}
                onChange={handleComposerChange}
                onKeyDown={handleComposerKeyDown}
                onPaste={handleComposerPaste}
                placeholder={`给 ${agent.name} 发消息；Enter 发送，Shift+Enter 换行`}
                rows={1}
                disabled={sending}
              />
              <button
                type="submit"
                className="composer-send"
                title={uploading ? "上传中…" : sending ? "发送中…" : "发送"}
                aria-label={uploading ? "上传中" : sending ? "发送中" : "发送"}
                disabled={!canSendDirectMessage(
                  draft,
                  pendingAttachments.length,
                  sending,
                )}
              >
                <ArrowUpIcon />
              </button>
            </div>
            <div className="composer-actions">
              <span className="composer-hint">
                Enter 发送、Shift+Enter 换行；支持拖放或粘贴图片；无需 @ 成员。
              </span>
            </div>
          </form>
        </div>
      </div>

        </div>

        <aside id="direct-side" className="direct-side" aria-label="对话侧栏">
          <div className="room-side-top">
            <span>对话面板</span>
            <button
              ref={sideCloseRef}
              type="button"
              className="btn btn-ghost btn-sm"
              onClick={() => setDirectSideOpen(false)}
            >
              关闭
            </button>
          </div>
          <DirectChatSideFacts
            agent={agent}
            room={room}
            connected={connected}
            runningCount={runningCount}
            permissionSaving={permissionSaving}
            onChangePermission={(mode) => void changePermissionMode(mode)}
          />
        </aside>
      </div>

      {directSideOpen && (
        <button
          type="button"
          className="direct-side-backdrop"
          aria-label="关闭对话侧栏"
          onClick={() => setDirectSideOpen(false)}
        />
      )}

      {sessionPanelOpen && (
        <SessionPanel
          roomId={roomId}
          members={[
            {
              id: agent.id,
              name: agent.name,
              builtinRole: agent.builtinRole,
            },
          ]}
          onClose={() => setSessionPanelOpen(false)}
        />
      )}

      {clearOpen && (
        <div className="modal-layer">
          <div
            className="modal-card"
            role="dialog"
            aria-modal="true"
            aria-labelledby="direct-clear-title"
          >
            <header className="modal-head">
              <h2 id="direct-clear-title" className="card-title">
                清屏
              </h2>
              <button
                type="button"
                className="btn btn-ghost btn-sm"
                aria-label="关闭清屏对话框"
                onClick={() => setClearOpen(false)}
                disabled={clearing}
              >
                关闭
              </button>
            </header>
            <p className="muted">
              清屏会删除这段直聊的全部纯聊天消息与附件；运行记录与任务不受影响。
              有运行中的任务时无法清屏。
            </p>
            {clearError !== null && (
              <p className="banner error" role="alert">
                {clearError}
              </p>
            )}
            <div className="modal-actions">
              <button
                type="button"
                className="btn btn-secondary"
                onClick={() => setClearOpen(false)}
                disabled={clearing}
              >
                取消
              </button>
              <button
                type="button"
                className="btn btn-danger"
                onClick={() => void submitClear()}
                disabled={clearing}
              >
                {clearing ? "清理中…" : "确认清屏"}
              </button>
            </div>
          </div>
        </div>
      )}

      {deleteOpen && (
        <DirectDeleteDialog
          agentName={agent.name}
          runningCount={runningCount}
          deleting={deleting}
          error={deleteError}
          onCancel={() => setDeleteOpen(false)}
          onConfirm={() => void confirmDelete()}
        />
      )}
    </section>
  );
}
