import { useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { api } from "../api/client";
import {
  buildCommandItems,
  filterCommands,
  nextIndex,
  type CommandAction,
  type CommandEntities,
  type CommandItem,
} from "./commandPalette";

interface CommandPaletteProps {
  open: boolean;
  onClose: () => void;
}

const EMPTY_ENTITIES: CommandEntities = { rooms: [], agents: [], tasks: [] };

/** 全局命令面板：Ctrl/Cmd+K 打开，搜索页面/房间/任务/智能体与新建动作。 */
export function CommandPalette({ open, onClose }: CommandPaletteProps) {
  const navigate = useNavigate();
  const [query, setQuery] = useState("");
  const [activeIndex, setActiveIndex] = useState(0);
  const [entities, setEntities] = useState<CommandEntities>(EMPTY_ENTITIES);
  const [loading, setLoading] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement | null>(null);

  const items = useMemo(() => buildCommandItems(entities), [entities]);
  const results = useMemo(() => filterCommands(items, query), [items, query]);

  // 每次打开重置查询、焦点与滚动；关闭时还原焦点。
  useEffect(() => {
    if (!open) {
      return;
    }
    const previous =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    setQuery("");
    setActiveIndex(0);
    inputRef.current?.focus();

    return () => {
      document.body.style.overflow = previousOverflow;
      previous?.focus();
    };
  }, [open]);

  // 打开时加载可搜索实体；单个数据源失败只隐藏对应分组。
  useEffect(() => {
    if (!open) {
      return;
    }
    let disposed = false;
    setLoading(true);
    setLoadError(null);
    void (async () => {
      const [roomsResult, agentsResult] = await Promise.allSettled([
        api.rooms.list(),
        api.agents.list(),
      ]);
      const rooms =
        roomsResult.status === "fulfilled" ? roomsResult.value : [];
      const agents =
        agentsResult.status === "fulfilled" ? agentsResult.value : [];
      const taskResults = await Promise.allSettled(
        rooms.map((room) => api.tasks.list(room.id)),
      );
      const tasks = taskResults.flatMap((result) =>
        result.status === "fulfilled" ? result.value : [],
      );
      if (disposed) {
        return;
      }
      setEntities({ rooms, agents, tasks });
      const failed = [roomsResult, agentsResult, ...taskResults].filter(
        (result) => result.status === "rejected",
      ).length;
      setLoadError(failed > 0 ? "部分结果加载失败，仍可搜索页面与动作" : null);
      setLoading(false);
    })();
    return () => {
      disposed = true;
    };
  }, [open]);

  useEffect(() => {
    setActiveIndex(0);
  }, [query]);

  if (!open) {
    return null;
  }

  const execute = (item: CommandItem | undefined) => {
    if (item === undefined) {
      return;
    }
    if (item.action !== undefined) {
      runAction(item.action, navigate);
    } else if (item.to !== undefined) {
      navigate(item.to);
    }
    onClose();
  };

  const handleKeyDown = (event: React.KeyboardEvent) => {
    if (event.key === "Escape") {
      event.preventDefault();
      onClose();
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
      execute(results[activeIndex]);
      return;
    }
    if (event.key === "Tab") {
      // 单输入面板：把 Tab 留在输入框，避免焦点跑到遮罩后的页面。
      event.preventDefault();
    }
  };

  return (
    <div
      className="command-layer"
      role="presentation"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) {
          onClose();
        }
      }}
    >
      <div
        className="command-card"
        role="dialog"
        aria-modal="true"
        aria-label="命令面板"
      >
        <input
          ref={inputRef}
          className="command-input"
          role="combobox"
          aria-expanded="true"
          aria-controls="command-palette-list"
          aria-autocomplete="list"
          aria-activedescendant={
            results.length > 0 ? `command-option-${activeIndex}` : undefined
          }
          placeholder="搜索页面、房间、任务、智能体，或输入动作…"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          onKeyDown={handleKeyDown}
        />
        <ul
          id="command-palette-list"
          className="command-list"
          role="listbox"
          aria-label="命令结果"
        >
          {results.map((item, index) => (
            <li key={item.id} role="presentation">
              <button
                id={`command-option-${index}`}
                type="button"
                role="option"
                aria-selected={index === activeIndex}
                className={
                  index === activeIndex
                    ? "command-option is-active"
                    : "command-option"
                }
                onMouseEnter={() => setActiveIndex(index)}
                onClick={() => execute(item)}
              >
                <span className="command-option-label">{item.label}</span>
                <span className="command-option-group">{item.group}</span>
              </button>
            </li>
          ))}
          {results.length === 0 && (
            <li className="command-empty" role="presentation">
              {loading ? "正在加载…" : "没有匹配结果"}
            </li>
          )}
        </ul>
        <footer className="command-hint">
          <span>↑↓ 选择</span>
          <span>Enter 打开</span>
          <span>Esc 关闭</span>
          {loadError !== null && <span className="command-warning">{loadError}</span>}
          {loadError === null && loading && <span>加载中…</span>}
        </footer>
      </div>
    </div>
  );
}

function runAction(action: CommandAction, navigate: ReturnType<typeof useNavigate>) {
  switch (action) {
    case "new-room":
      navigate("/rooms?new=1");
      break;
    case "new-agent":
      navigate("/agents?new=1");
      break;
    case "new-task":
      navigate("/tasks?new=1");
      break;
    case "settings":
      navigate("/settings");
      break;
  }
}
