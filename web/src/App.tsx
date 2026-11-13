import { useEffect, useRef, useState } from "react";
import {
  BrowserRouter,
  Link,
  Navigate,
  Route,
  Routes,
  useLocation,
} from "react-router-dom";
import type { Window as TauriWindow } from "@tauri-apps/api/window";
import AgentsPage from "./pages/AgentsPage";
import { isDesktopShell } from "./desktopShell";
import { useTheme } from "./useTheme";
import AgentBuilderPage from "./pages/AgentBuilderPage";
import HarnessesPage from "./pages/HarnessesPage";
import RoomPage from "./pages/RoomPage";
import RoomsPage from "./pages/RoomsPage";
import RunPage from "./pages/RunPage";
import SessionsPage from "./pages/SessionsPage";
import SettingsPage from "./pages/SettingsPage";
import TasksPage from "./pages/TasksPage";
import { AppSidebar } from "./components/layout/AppSidebar";
import {
  RoomChromeProvider,
  resolveTopbarTitle,
  useRoomChrome,
} from "./components/layout/roomChrome";
import { CommandPalette } from "./components/CommandPaletteDialog";
import UpdateBanner from "./components/UpdateBanner";
import {
  activeSection,
  readSidebarCollapsed,
  sectionLabel,
  writeSidebarCollapsed,
} from "./components/layout/shellNav";
import { useNavSummary } from "./useNavSummary";

/** 内联图标：统一 stroke=currentColor，避免引入图标库或外部资源。 */
function MoonIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      <path d="M20.5 14.6A8.5 8.5 0 1 1 9.4 3.5a6.8 6.8 0 0 0 11.1 11.1Z" />
    </svg>
  );
}

function SunIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      <circle cx="12" cy="12" r="4" />
      <path d="M12 2v2" />
      <path d="M12 20v2" />
      <path d="M4.9 4.9l1.4 1.4" />
      <path d="M17.7 17.7l1.4 1.4" />
      <path d="M2 12h2" />
      <path d="M20 12h2" />
      <path d="M4.9 19.1l1.4-1.4" />
      <path d="M17.7 6.3l1.4-1.4" />
    </svg>
  );
}

function MenuIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      <path d="M4 7h16" />
      <path d="M4 12h16" />
      <path d="M4 17h16" />
    </svg>
  );
}

/** 顶栏返回图标。 */
function BackIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      <path d="M14.5 5 8 12l6.5 7" />
    </svg>
  );
}

/**
 * 页头主题切换：亮色时显示月亮（切到暗色），暗色时显示太阳（切到亮色）。
 *
 * 初始主题由 useTheme 按存储/系统偏好解析，index.html 的内联脚本已在首帧前
 * 写入 data-theme，这里只负责后续切换。
 */
function ThemeToggle() {
  const [theme, toggle] = useTheme();
  const goesDark = theme === "light";
  const label = goesDark ? "切换为暗色主题" : "切换为亮色主题";

  return (
    <button
      type="button"
      className="theme-toggle"
      onClick={toggle}
      aria-label={label}
      title={label}
    >
      {goesDark ? <MoonIcon /> : <SunIcon />}
    </button>
  );
}

/** 窗口控制图标：10×10 线性描边，与页头主题按钮同源使用 currentColor。 */
function MinimizeIcon() {
  return (
    <svg
      viewBox="0 0 10 10"
      fill="none"
      stroke="currentColor"
      strokeWidth="1"
      aria-hidden="true"
      focusable="false"
    >
      <path d="M1.5 5h7" />
    </svg>
  );
}

function MaximizeIcon() {
  return (
    <svg
      viewBox="0 0 10 10"
      fill="none"
      stroke="currentColor"
      strokeWidth="1"
      aria-hidden="true"
      focusable="false"
    >
      <rect x="1.5" y="1.5" width="7" height="7" rx="0.5" />
    </svg>
  );
}

function RestoreIcon() {
  return (
    <svg
      viewBox="0 0 10 10"
      fill="none"
      stroke="currentColor"
      strokeWidth="1"
      aria-hidden="true"
      focusable="false"
    >
      <rect x="1.5" y="3.5" width="5" height="5" rx="0.5" />
      <path d="M3.5 3.5v-2h5v5h-2" />
    </svg>
  );
}

function CloseIcon() {
  return (
    <svg
      viewBox="0 0 10 10"
      fill="none"
      stroke="currentColor"
      strokeWidth="1"
      aria-hidden="true"
      focusable="false"
    >
      <path d="M2 2l6 6" />
      <path d="M8 2l-6 6" />
    </svg>
  );
}

/**
 * 桌面窗口控制：最小化 / 最大化(还原) / 关闭。
 *
 * 只在 Tauri 桌面壳内渲染（浏览器模式返回 null）；Tauri API 用动态导入，
 * 避免浏览器构建把窗口模块带到首屏。maximized 由 onResized 与系统状态同步。
 */
function WindowControls() {
  const desktop = isDesktopShell();
  const windowRef = useRef<TauriWindow | null>(null);
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    if (!desktop) {
      return;
    }

    let disposed = false;
    let unlisten: (() => void) | null = null;

    const syncMaximized = (win: TauriWindow) => {
      void win.isMaximized().then((value) => {
        if (!disposed) {
          setMaximized(value);
        }
      });
    };

    void (async () => {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      if (disposed) {
        return;
      }
      const win = getCurrentWindow();
      windowRef.current = win;
      syncMaximized(win);
      const stop = await win.onResized(() => syncMaximized(win));
      if (disposed) {
        stop();
      } else {
        unlisten = stop;
      }
    })();

    return () => {
      disposed = true;
      windowRef.current = null;
      unlisten?.();
    };
  }, [desktop]);

  if (!desktop) {
    return null;
  }

  const toggleLabel = maximized ? "还原" : "最大化";

  return (
    <div className="window-controls">
      <span className="window-controls-divider" aria-hidden="true" />
      <button
        type="button"
        className="window-control"
        onClick={() => void windowRef.current?.minimize()}
        aria-label="最小化"
        title="最小化"
      >
        <MinimizeIcon />
      </button>
      <button
        type="button"
        className="window-control"
        onClick={() => void windowRef.current?.toggleMaximize()}
        aria-label={toggleLabel}
        title={toggleLabel}
      >
        {maximized ? <RestoreIcon /> : <MaximizeIcon />}
      </button>
      <button
        type="button"
        className="window-control close"
        onClick={() => void windowRef.current?.close()}
        aria-label="关闭"
        title="关闭"
      >
        <CloseIcon />
      </button>
    </div>
  );
}

function Brand() {
  return (
    <div className="app-brand">
      <span className="app-brand-mark">
        <MoonIcon />
      </span>
      <span className="app-brand-text">
        <span className="app-brand-title">Moon</span>
        <span className="app-brand-sep" aria-hidden="true">
          ·
        </span>
        <span className="app-brand-sub">多智能体协作工作台</span>
      </span>
    </div>
  );
}

function useNarrowShell(): boolean {
  const [narrow, setNarrow] = useState<boolean>(() =>
    typeof window === "undefined"
      ? false
      : window.matchMedia("(max-width: 899px)").matches,
  );

  useEffect(() => {
    const media = window.matchMedia("(max-width: 899px)");
    const sync = () => setNarrow(media.matches);
    sync();
    // 某些嵌入环境先以占位宽度挂载、随后才应用真实视口，且不派发 media change；
    // 首帧补一次并监听窗口 resize，保证侧栏与抽屉状态跟真实宽度一致。
    const frame = window.requestAnimationFrame(sync);
    media.addEventListener("change", sync);
    window.addEventListener("resize", sync);
    return () => {
      window.cancelAnimationFrame(frame);
      media.removeEventListener("change", sync);
      window.removeEventListener("resize", sync);
    };
  }, []);

  return narrow;
}

/**
 * 应用外壳：左侧导航 + 顶部状态栏 + 独立滚动主区。
 *
 * 桌面宽度侧栏常驻并可折叠为图标栏；窄窗口退化为可关闭抽屉。
 * 路由保持不变，页面内 PageHeader 继续承担页面级标题与动作。
 */
function AppShell() {
  const location = useLocation();
  const narrow = useNarrowShell();
  const [collapsed, setCollapsed] = useState<boolean>(() =>
    readSidebarCollapsed(),
  );
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const menuButtonRef = useRef<HTMLButtonElement | null>(null);
  const summary = useNavSummary(location.pathname);
  const section = activeSection(location.pathname);
  const { chrome } = useRoomChrome();

  // 路由变化或回到桌面宽度时收起抽屉。
  useEffect(() => {
    setDrawerOpen(false);
  }, [location.pathname, narrow]);

  // 全局命令面板：Ctrl/Cmd+K 在任意页面打开。
  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (
        (event.ctrlKey || event.metaKey) &&
        event.key.toLowerCase() === "k"
      ) {
        event.preventDefault();
        setPaletteOpen(true);
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, []);

  useEffect(() => {
    if (!drawerOpen) {
      return;
    }

    const focusables = () =>
      Array.from(
        document.querySelectorAll<HTMLElement>(
          "#app-sidebar a[href], #app-sidebar button:not([disabled])",
        ),
      );

    focusables()[0]?.focus();

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        setDrawerOpen(false);
        return;
      }
      if (event.key !== "Tab") {
        return;
      }
      const items = focusables();
      if (items.length === 0) {
        return;
      }
      const first = items[0];
      const last = items[items.length - 1];
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
      // 用户主动关闭时焦点回到菜单按钮；跨 900px 边界导致按钮卸载时退回主内容。
      if (menuButtonRef.current !== null) {
        menuButtonRef.current.focus();
      } else {
        document.getElementById("main-content")?.focus();
      }
    };
  }, [drawerOpen]);

  const toggleCollapsed = () => {
    const next = !collapsed;
    setCollapsed(next);
    writeSidebarCollapsed(next);
  };

  const closeDrawer = () => setDrawerOpen(false);

  return (
    <div className={`app-shell${collapsed ? " is-collapsed" : ""}`}>
      <a className="skip-link" href="#main-content">
        跳到主内容
      </a>

      {!narrow && (
        <AppSidebar
          brand={<Brand />}
          summary={summary}
          collapsed={collapsed}
          onToggleCollapsed={toggleCollapsed}
          onNavigate={closeDrawer}
        />
      )}

      <div className="app-frame">
        <header className="app-topbar" data-tauri-drag-region="deep">
          <div className="app-topbar-left">
            {narrow && (
              <button
                ref={menuButtonRef}
                type="button"
                className="app-menu-button"
                aria-label="打开导航"
                aria-controls="app-sidebar"
                aria-expanded={drawerOpen}
                title="打开导航"
                onClick={() => setDrawerOpen(true)}
              >
                <MenuIcon />
              </button>
            )}
            {chrome !== null && (
              <Link
                className="app-topbar-back"
                to={chrome.backTo}
                aria-label={chrome.backLabel}
                title={chrome.backLabel}
              >
                <BackIcon />
              </Link>
            )}
            <span
              className={
                chrome !== null
                  ? "app-topbar-title is-room"
                  : "app-topbar-title"
              }
            >
              {resolveTopbarTitle(chrome, sectionLabel(section))}
            </span>
            {chrome?.meta !== undefined && chrome.meta !== "" && (
              <span className="app-topbar-meta" title={chrome.meta}>
                {chrome.meta}
              </span>
            )}
          </div>

          <div className="app-header-actions">
            <ThemeToggle />
            <WindowControls />
          </div>
        </header>

        <main className="app-main" id="main-content" tabIndex={-1}>
          <UpdateBanner />
          <Routes>
            <Route path="/" element={<Navigate to="/agents" replace />} />
            <Route path="/agents" element={<AgentsPage />} />
            <Route path="/agents/builder" element={<AgentBuilderPage />} />
            <Route path="/harnesses" element={<HarnessesPage />} />
            <Route path="/rooms" element={<RoomsPage />} />
            <Route path="/rooms/:id" element={<RoomPage />} />
            <Route path="/tasks" element={<TasksPage />} />
            <Route path="/sessions" element={<SessionsPage />} />
            <Route path="/settings" element={<SettingsPage />} />
            <Route path="/rooms/:id/runs" element={<RunPage />} />
            <Route path="/rooms/:id/runs/:runId" element={<RunPage />} />
            <Route path="*" element={<Navigate to="/agents" replace />} />
          </Routes>
        </main>
      </div>

      {narrow && drawerOpen && (
        <>
          <div
            className="app-drawer-backdrop"
            aria-hidden="true"
            onClick={closeDrawer}
          />
          <AppSidebar
            brand={<Brand />}
            summary={summary}
            variant="drawer"
            onNavigate={closeDrawer}
            onClose={closeDrawer}
          />
        </>
      )}

      <CommandPalette
        open={paletteOpen}
        onClose={() => setPaletteOpen(false)}
      />
    </div>
  );
}

/**
 * 应用根组件：注册路由与外壳。
 *
 * 服务端状态由各页面通过 API client 自行获取，不在此处提升为全局状态。
 */
export default function App() {
  return (
    <BrowserRouter>
      <RoomChromeProvider>
        <AppShell />
      </RoomChromeProvider>
    </BrowserRouter>
  );
}
