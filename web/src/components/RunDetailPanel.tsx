import { lazy, Suspense } from "react";
import type {
  Agent,
  AgentRun,
  Message,
  RunActivity,
  RunDetailResponse,
} from "@moon/shared";
import {
  activityDetailPreview,
  activityKindLabel,
  buildRunList,
  buildRunTimeline,
  describeAgentModel,
  formatActivityOffset,
  formatRunDuration,
  runStatusLabel,
  type RunTimelineEntry,
} from "./runInspectorView";
import { formatDiscussionConclusion } from "./discussionView";
import { formatUsage } from "./contextView";

// Markdown 依赖体积较大，按需加载，避免进入运行页主 chunk。
const MarkdownContent = lazy(() => import("./MarkdownContent"));

export interface ActiveRunSummary {
  runId: number;
  agentId: number;
  startedAt: number;
}

export interface RunDetailPanelProps {
  activeRuns: readonly ActiveRunSummary[];
  recentRuns: readonly AgentRun[];
  selectedRunId: number | null;
  detail: RunDetailResponse | null;
  /** 已合并服务端快照与实时推送的活动。 */
  activities: readonly RunActivity[];
  /** 该运行的阶段总结与最终回复；旧运行可能为空。 */
  messages: readonly Message[];
  loading: boolean;
  error: string | null;
  now: number;
  agentById: ReadonlyMap<number, Agent>;
  onSelect: (runId: number) => void;
}

function formatClock(ms: number): string {
  return new Date(ms).toLocaleTimeString("zh-CN", { hour12: false });
}

function formatDateTime(ms: number): string {
  return new Date(ms).toLocaleString("zh-CN", { hour12: false });
}

type TimelineBlock =
  | { kind: "message"; entry: Extract<RunTimelineEntry, { kind: "message" }> }
  | {
      kind: "activities";
      entries: Array<Extract<RunTimelineEntry, { kind: "activity" }>>;
    };

/** 连续活动合并成一个可折叠过程块，避免工具调用把对话气泡冲散。 */
function groupTimelineBlocks(entries: readonly RunTimelineEntry[]): TimelineBlock[] {
  const blocks: TimelineBlock[] = [];
  for (const entry of entries) {
    if (entry.kind === "message") {
      blocks.push({ kind: "message", entry });
      continue;
    }
    const last = blocks.at(-1);
    if (last !== undefined && last.kind === "activities") {
      last.entries.push(entry);
    } else {
      blocks.push({ kind: "activities", entries: [entry] });
    }
  }
  return blocks;
}

/**
 * 运行详情面板：左侧是最近运行，右侧是所选运行的指令、活动与输出。
 *
 * 只展示服务端已落定的事实，不自行解析 harness 原始事件；页面与未来的
 * 其它容器共用这份展示逻辑，选中状态由调用方决定。
 */
export function RunDetailPanel({
  activeRuns,
  recentRuns,
  selectedRunId,
  detail,
  activities,
  messages,
  loading,
  error,
  now,
  agentById,
  onSelect,
}: RunDetailPanelProps) {
  const items = buildRunList(activeRuns, recentRuns);
  const selectedRun = detail?.run ?? null;
  const usage =
    selectedRun === null
      ? { tokens: null, context: null }
      : formatUsage(selectedRun);
  const selectedAgent =
    selectedRun === null ? undefined : agentById.get(selectedRun.agentId);
  const selectedActive = activeRuns.some(
    (run) => run.runId === selectedRunId,
  );
  const running =
    selectedRun === null ? selectedActive : selectedRun.status === "running";
  const timeline = buildRunTimeline(messages, activities);
  const blocks = groupTimelineBlocks(timeline);
  const hasConversation = messages.length > 0;

  return (
    <div className="run-detail-layout">
      <section className="run-list-pane" aria-label="最近运行">
        <h2 className="run-pane-title">最近运行</h2>
        {items.length === 0 ? (
          <p className="muted small">这个房间还没有运行记录。</p>
        ) : (
          <ul className="run-list">
            {items.map((item) => {
              const agent = agentById.get(item.agentId);
              const active =
                item.active ||
                item.run === null ||
                item.run.status === "running";
              const label =
                item.run === null ? "运行中" : runStatusLabel(item.run.status);
              return (
                <li key={item.runId}>
                  <button
                    type="button"
                    className={
                      item.runId === selectedRunId
                        ? "run-list-item is-active"
                        : "run-list-item"
                    }
                    aria-pressed={item.runId === selectedRunId}
                    onClick={() => onSelect(item.runId)}
                  >
                    <span className="run-list-main">
                      <span className="run-list-name">
                        {agent?.name ?? `智能体#${item.agentId}`}
                      </span>
                      <span
                        className={`status-pill run-status ${active ? "running" : item.run!.status}`}
                      >
                        {label}
                      </span>
                    </span>
                    <span className="run-list-meta">
                      #{item.runId} ·{" "}
                      {active
                        ? formatClock(item.startedAt)
                        : formatClock(item.run!.createdAt)}{" "}
                      ·{" "}
                      {item.run === null
                        ? "进行中"
                        : formatRunDuration(item.run, now)}
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>
        )}
      </section>

      <section className="run-detail-pane" aria-label="运行明细">
        {error !== null && (
          <p className="banner error" role="alert">
            {error}
          </p>
        )}

        {selectedRunId === null ? (
          <p className="muted">选择一条运行查看详情。</p>
        ) : loading && detail === null ? (
          <p className="muted">加载运行详情…</p>
        ) : selectedRun === null ? (
          <p className="muted">
            {error === null ? "正在加载运行记录…" : "无法读取这次运行的记录。"}
          </p>
        ) : (
          <>
            <header className="run-detail-head">
              <div>
                <h2 className="run-detail-title">
                  {selectedAgent?.name ?? `智能体#${selectedRun.agentId}`}
                </h2>
                <p className="run-detail-sub">
                  {describeAgentModel(selectedAgent)} · #{selectedRun.id}
                </p>
              </div>
              {running && (
                <span className="msg-generating">
                  <span className="pulse-dot" aria-hidden="true" />
                  运行中
                </span>
              )}
            </header>

            <dl className="detail-fields">
              <div className="detail-row">
                <dt>状态</dt>
                <dd>
                  <span
                    className={`status-pill run-status ${selectedRun.status}`}
                  >
                    {runStatusLabel(selectedRun.status)}
                  </span>
                </dd>
              </div>
              <div className="detail-row">
                <dt>开始</dt>
                <dd>{formatDateTime(selectedRun.createdAt)}</dd>
              </div>
              <div className="detail-row">
                <dt>耗时</dt>
                <dd>{formatRunDuration(selectedRun, now)}</dd>
              </div>
              {usage.tokens !== null && (
                <div className="detail-row">
                  <dt>tokens</dt>
                  <dd>{selectedRun.tokenCount}</dd>
                </div>
              )}
              {usage.context !== null && (
                <div className="detail-row">
                  <dt>上下文</dt>
                  <dd>{usage.context}</dd>
                </div>
              )}
              {selectedRun.taskId !== null && (
                <div className="detail-row">
                  <dt>任务</dt>
                  <dd>#{selectedRun.taskId}</dd>
                </div>
              )}
            </dl>

            <div className="run-conversation" aria-label="运行对话">
              <section className="run-turn is-input">
                <header className="run-turn-head">
                  <span className="run-turn-role">输入指令</span>
                  <span className="run-turn-meta">
                    {selectedRun.prompt.length} 字
                  </span>
                </header>
                <details className="run-input-details">
                  <summary>查看完整输入</summary>
                  <pre className="run-prompt">{selectedRun.prompt}</pre>
                </details>
              </section>

              {blocks.map((block) =>
                block.kind === "message" ? (
                  <article
                    key={block.entry.key}
                    className={`run-turn is-${block.entry.message.kind} is-${block.entry.message.status}`}
                  >
                    <header className="run-turn-head">
                      <span className="run-turn-role">
                        {block.entry.message.kind === "progress"
                          ? "阶段总结"
                          : "最终结论"}
                      </span>
                      <span className="run-turn-meta">
                        {formatClock(block.entry.message.createdAt)}
                      </span>
                    </header>
                    {block.entry.message.kind === "progress" ? (
                      <p className="run-turn-text">
                        {block.entry.message.content}
                      </p>
                    ) : (
                      <div className="run-turn-markdown">
                        <Suspense
                          fallback={
                            <p className="msg-markdown-fallback">
                              正在渲染消息…
                            </p>
                          }
                        >
                          <MarkdownContent
                            text={formatDiscussionConclusion(
                              block.entry.message.content,
                              block.entry.message.discussion != null &&
                                block.entry.message.discussion.stance === null
                                ? "judge"
                                : null,
                            )}
                          />
                        </Suspense>
                      </div>
                    )}
                  </article>
                ) : (
                  <details
                    key={block.entries[0]!.key}
                    className="run-process"
                  >
                    <summary>
                      工具与思考
                      <span className="run-section-count">
                        {block.entries.length}
                      </span>
                    </summary>
                    <ol className="run-activity-list">
                      {block.entries.map(({ activity }) => {
                        const preview = activityDetailPreview(activity.detail);
                        return (
                          <li
                            key={activity.seq}
                            className={`run-activity is-${activity.kind}`}
                          >
                            <span className="run-activity-kind">
                              {activityKindLabel(activity.kind)}
                            </span>
                            <div className="run-activity-main">
                              <div className="run-activity-head">
                                <strong>{activity.title}</strong>
                                <span className="run-activity-time">
                                  {formatActivityOffset(
                                    activity.at,
                                    selectedRun.createdAt,
                                  )}
                                </span>
                              </div>
                              {preview.text !== "" &&
                                (preview.truncated ? (
                                  <details className="run-activity-detail">
                                    <summary>{preview.text}</summary>
                                    <pre>{activity.detail}</pre>
                                  </details>
                                ) : (
                                  <p className="run-activity-detail">
                                    {preview.text}
                                  </p>
                                ))}
                            </div>
                          </li>
                        );
                      })}
                    </ol>
                  </details>
                ),
              )}

              {!hasConversation && (
                <section className="run-turn is-reply">
                  <header className="run-turn-head">
                    <span className="run-turn-role">输出</span>
                  </header>
                  {selectedRun.result !== null &&
                  selectedRun.result.trim() !== "" ? (
                    <div className="run-turn-markdown">
                      <Suspense
                        fallback={
                          <p className="msg-markdown-fallback">
                            正在渲染消息…
                          </p>
                        }
                      >
                        <MarkdownContent text={selectedRun.result} />
                      </Suspense>
                    </div>
                  ) : (
                    <p className="muted small">
                      {running
                        ? "运行尚未结束，暂无输出。"
                        : "这次运行没有返回文本。"}
                    </p>
                  )}
                </section>
              )}

              {running && hasConversation && (
                <p className="run-typing">
                  <span className="pulse-dot" aria-hidden="true" />
                  正在继续执行…
                </p>
              )}

              {selectedRun.error !== null && (
                <p className="banner error" role="alert">
                  {selectedRun.error}
                </p>
              )}
            </div>
          </>
        )}
      </section>
    </div>
  );
}
