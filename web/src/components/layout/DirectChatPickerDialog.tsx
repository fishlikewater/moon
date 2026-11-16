import { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import type { Agent } from "@moon/shared";
import { AgentAvatar } from "../AgentAvatar";
import { nextIndex } from "../commandPalette";
import {
  buildDirectChatPickerItems,
  directChatPickerListState,
  filterDirectChatPickerItems,
} from "./directChatPicker";

interface DirectChatPickerDialogProps {
  agents: readonly Agent[];
  /** 智能体列表是否仍在加载。 */
  loading: boolean;
  /** 智能体列表加载失败原因；null 表示成功。 */
  loadError: string | null;
  /** 已有直聊的智能体 id，用于「已有直聊」标记。 */
  existingAgentIds: ReadonlySet<number>;
  /** 创建请求进行中：禁用候选项并提示。 */
  busy: boolean;
  error: string | null;
  onPick: (agentId: number) => void;
  onClose: () => void;
}

const FOCUSABLE_SELECTOR = [
  "button:not([disabled])",
  "input:not([disabled])",
  "select:not([disabled])",
  "a[href]",
  '[tabindex]:not([tabindex="-1"])',
].join(",");

/**
 * 发起直聊的智能体选择器。
 *
 * 居中弹层 + 搜索 + 键盘选择；候选展示人设与模型，已有直聊会明确标记。
 * 焦点进入搜索框，Esc 关闭，Tab 在弹层内循环。
 */
export function DirectChatPickerDialog({
  agents,
  loading,
  loadError,
  existingAgentIds,
  busy,
  error,
  onPick,
  onClose,
}: DirectChatPickerDialogProps) {
  const [query, setQuery] = useState("");
  const [activeIndex, setActiveIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const dialogRef = useRef<HTMLDivElement | null>(null);
  const closeRef = useRef(onClose);
  const busyRef = useRef(busy);

  useEffect(() => {
    closeRef.current = onClose;
  }, [onClose]);

  useEffect(() => {
    busyRef.current = busy;
  }, [busy]);

  const items = useMemo(
    () => buildDirectChatPickerItems(agents, existingAgentIds),
    [agents, existingAgentIds],
  );
  const results = useMemo(
    () => filterDirectChatPickerItems(items, query),
    [items, query],
  );
  const listState = directChatPickerListState({
    loading,
    error: loadError,
    resultCount: results.length,
  });

  useEffect(() => {
    setActiveIndex(0);
  }, [query]);

  // 挂 body、锁滚动、焦点进入搜索框；关闭时归还触发按钮并恢复滚动。
  useEffect(() => {
    const previous =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    inputRef.current?.focus();

    const handleKeyDown = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape") {
        if (busyRef.current) {
          return;
        }
        event.preventDefault();
        closeRef.current();
        return;
      }
      if (event.key !== "Tab") {
        return;
      }
      const focusables =
        dialogRef.current?.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR);
      if (focusables === undefined || focusables.length === 0) {
        return;
      }
      const first = focusables[0];
      const last = focusables[focusables.length - 1];
      const active = document.activeElement;
      if (event.shiftKey && active === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && active === last) {
        event.preventDefault();
        first.focus();
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("keydown", handleKeyDown);
      document.body.style.overflow = previousOverflow;
      previous?.focus();
    };
  }, []);

  const pick = (agentId: number | undefined) => {
    if (busy || agentId === undefined) {
      return;
    }
    onPick(agentId);
  };

  const handleSearchKeyDown = (event: React.KeyboardEvent) => {
    if (listState !== "results") {
      return;
    }
    if (event.key === "ArrowDown") {
      event.preventDefault();
      setActiveIndex((current) => nextIndex(current, results.length, 1));
      return;
    }
    if (event.key === "ArrowUp") {
      event.preventDefault();
      setActiveIndex((current) => nextIndex(current, results.length, -1));
      return;
    }
    if (event.key === "Enter") {
      event.preventDefault();
      pick(results[activeIndex]?.agent.id);
    }
  };

  return createPortal(
    <div className="modal-layer">
      <div
        className="modal-card direct-picker-card"
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby="direct-picker-title"
      >
        <header className="modal-head">
          <div>
            <h2 id="direct-picker-title" className="card-title">
              发起直聊
            </h2>
            <p className="field-hint">
              选择智能体开始 1:1 对话；已有直聊会直接复用原会话。
            </p>
          </div>
          <button
            type="button"
            className="btn btn-ghost btn-sm"
            aria-label="关闭选择智能体"
            onClick={onClose}
            disabled={busy}
          >
            关闭
          </button>
        </header>

        <input
          ref={inputRef}
          className="direct-picker-search"
          type="search"
          role="combobox"
          aria-expanded="true"
          aria-controls="direct-picker-list"
          aria-autocomplete="list"
          aria-label="搜索智能体"
          aria-activedescendant={
            listState === "results" && results.length > 0
              ? `direct-picker-option-${activeIndex}`
              : undefined
          }
          placeholder="搜索名称、人设、harness 或模型…"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          onKeyDown={handleSearchKeyDown}
        />

        {error !== null && (
          <p className="banner error" role="alert">
            {error}
          </p>
        )}

        <ul
          id="direct-picker-list"
          className="direct-picker-list"
          role="listbox"
          aria-label="智能体列表"
          aria-busy={loading}
        >
          {listState === "loading" && (
            <li className="direct-picker-empty" role="presentation">
              正在加载智能体…
            </li>
          )}
          {listState === "error" && (
            <li
              className="direct-picker-empty direct-picker-load-error"
              role="presentation"
            >
              智能体列表加载失败：{loadError}
            </li>
          )}
          {listState === "empty" && (
            <li className="direct-picker-empty" role="presentation">
              {query.trim() === ""
                ? "还没有智能体，先到智能体页创建"
                : "没有匹配的智能体"}
            </li>
          )}
          {listState === "results" &&
            results.map((item, index) => {
              const agent = item.agent;
              const model = agent.modelConfig?.model ?? "默认";
              const reasoning = agent.modelConfig?.reasoningLevel;
              return (
                <li key={agent.id} role="presentation">
                  <button
                    id={`direct-picker-option-${index}`}
                    type="button"
                    role="option"
                    aria-selected={index === activeIndex}
                    className={
                      index === activeIndex
                        ? "direct-picker-option is-active"
                        : "direct-picker-option"
                    }
                    disabled={busy}
                    onMouseEnter={() => setActiveIndex(index)}
                    onClick={() => pick(agent.id)}
                  >
                    <AgentAvatar id={agent.id} name={agent.name} size="md" />
                    <span className="direct-picker-option-body">
                      <span className="direct-picker-option-head">
                        <span className="direct-picker-option-name">
                          {agent.name}
                        </span>
                        <span className="badge badge-harness">
                          {agent.harness}
                        </span>
                        {item.existing && (
                          <span className="direct-picker-option-existing">
                            已有直聊
                          </span>
                        )}
                      </span>
                      <span className="chip chip-mono direct-picker-option-model">
                        {model}
                        {typeof reasoning === "string" &&
                        reasoning.trim() !== ""
                          ? ` · ${reasoning}`
                          : ""}
                      </span>
                      <span className="direct-picker-option-persona">
                        {agent.persona}
                      </span>
                    </span>
                    <span
                      className="direct-picker-option-enter"
                      aria-hidden="true"
                    >
                      ↵
                    </span>
                  </button>
                </li>
              );
            })}
        </ul>

        <footer className="direct-picker-hint">
          <span>↑↓ 选择</span>
          <span>Enter 开始对话</span>
          <span>Esc 关闭</span>
          {busy && <span className="direct-picker-hint-busy">正在打开对话…</span>}
        </footer>
      </div>
    </div>,
    document.body,
  );
}
