import { useCallback, useEffect, useState } from "react";

/**
 * 亮/暗主题状态：首访跟随系统，切换后持久化到 localStorage。
 *
 * index.html 的内联脚本在 React 挂载前按同一规则设置 data-theme（防闪烁），
 * 本 hook 挂载后接管后续切换。解析逻辑收敛在纯函数上，便于在 node
 * 测试环境下覆盖分支。
 */

export type Theme = "light" | "dark";

/** 用户偏好：显式亮/暗，或跟随系统。 */
export type ThemePreference = Theme | "system";

export const THEME_STORAGE_KEY = "moon-theme";

/** 同页多实例（页头按钮与设置页）同步主题变化。 */
export const THEME_CHANGE_EVENT = "moon-theme-change";

/** 存储读取的最小接口，避免测试依赖真实 localStorage。 */
type StorageLike = Pick<Storage, "getItem">;

/** matchMedia 的最小接口。 */
type MediaLike = (query: string) => { matches: boolean };

/** 读取持久化主题；非法值视为未存储。 */
export function readStoredTheme(storage: StorageLike | null): Theme | null {
  try {
    const stored = storage?.getItem(THEME_STORAGE_KEY);
    return stored === "light" || stored === "dark" ? stored : null;
  } catch {
    return null;
  }
}

/** 读取系统偏好；媒体查询不可用时回退亮色。 */
export function systemTheme(matchMedia: MediaLike | null): Theme {
  try {
    return matchMedia?.("(prefers-color-scheme: dark)")?.matches
      ? "dark"
      : "light";
  } catch {
    return "light";
  }
}

/** 初始主题：存储优先，其次系统偏好。 */
export function resolveInitialTheme(
  storage: StorageLike | null,
  matchMedia: MediaLike | null,
): Theme {
  return readStoredTheme(storage) ?? systemTheme(matchMedia);
}

/** 切换目标。 */
export function nextTheme(current: Theme): Theme {
  return current === "dark" ? "light" : "dark";
}

/** 把主题写入根元素；测试环境无 document 时为空操作。 */
export function applyTheme(theme: Theme): void {
  if (typeof document === "undefined") {
    return;
  }
  document.documentElement.dataset.theme = theme;
}

/**
 * 写入主题偏好并立即应用：`system` 会移除存储键，回到跟随系统。
 *
 * 通过自定义事件同步同页其它 `useTheme` 实例，避免页头按钮与设置页状态分叉。
 */
export function writeThemePreference(preference: ThemePreference): Theme {
  const resolved =
    preference === "system"
      ? systemTheme(typeof window === "undefined" ? null : window.matchMedia)
      : preference;
  try {
    if (preference === "system") {
      window.localStorage.removeItem(THEME_STORAGE_KEY);
    } else {
      window.localStorage.setItem(THEME_STORAGE_KEY, preference);
    }
  } catch {
    // 持久化失败不阻塞本次切换。
  }
  applyTheme(resolved);
  if (typeof window !== "undefined") {
    window.dispatchEvent(new Event(THEME_CHANGE_EVENT));
  }
  return resolved;
}

export function useTheme(): [Theme, () => void] {
  const [theme, setTheme] = useState<Theme>(() =>
    resolveInitialTheme(
      typeof window === "undefined" ? null : window.localStorage,
      typeof window === "undefined" ? null : window.matchMedia,
    ),
  );

  useEffect(() => {
    applyTheme(theme);
  }, [theme]);

  useEffect(() => {
    if (typeof window === "undefined") {
      return;
    }
    const sync = () =>
      setTheme(
        resolveInitialTheme(window.localStorage, window.matchMedia),
      );
    window.addEventListener(THEME_CHANGE_EVENT, sync);
    return () => window.removeEventListener(THEME_CHANGE_EVENT, sync);
  }, []);

  const toggle = useCallback(() => {
    const next = nextTheme(theme);
    // 副作用不能放在 setState 更新器里：严格模式下更新器会被重复调用，
    // 事件回灌会把状态再翻一次，最终写回旧主题。
    writeThemePreference(next);
    setTheme(next);
  }, [theme]);

  return [theme, toggle];
}
