import { afterEach, describe, expect, it, vi } from "vitest";
import {
  applyTheme,
  nextTheme,
  readStoredTheme,
  resolveInitialTheme,
  systemTheme,
} from "./useTheme";

/**
 * 主题状态行为（node 环境，不依赖 jsdom / testing-library）。
 *
 * useTheme 的运行时分支全部收敛在导出的纯函数上：存储读取、系统偏好、
 * 初始解析、切换与根元素写入。这里用受控 fake 覆盖这些分支，钩子本身
 * 只剩 useState/useEffect 的组合。
 */

const STORAGE_KEY = "moon-theme";

/** 最小存储替身：只实现 getItem。 */
function fakeStorage(value: string | null): { getItem: (key: string) => string | null } {
  return { getItem: (key) => (key === STORAGE_KEY ? value : null) };
}

/** 最小 matchMedia 替身。 */
function fakeMedia(prefersDark: boolean): (query: string) => { matches: boolean } {
  return (query) => ({
    matches: prefersDark && query === "(prefers-color-scheme: dark)",
  });
}

describe("readStoredTheme", () => {
  it("读取合法值 light/dark", () => {
    expect(readStoredTheme(fakeStorage("light"))).toBe("light");
    expect(readStoredTheme(fakeStorage("dark"))).toBe("dark");
  });

  it("无存储、非法值与非主题键都视为未存储", () => {
    expect(readStoredTheme(null)).toBeNull();
    expect(readStoredTheme(fakeStorage(null))).toBeNull();
    expect(readStoredTheme(fakeStorage("sepia"))).toBeNull();
    expect(
      readStoredTheme({ getItem: (key) => (key === "other" ? "dark" : null) }),
    ).toBeNull();
  });

  it("getItem 抛异常时退化为未存储且不向上冒泡", () => {
    const throwing = {
      getItem: () => {
        throw new Error("storage unavailable");
      },
    };
    expect(() => readStoredTheme(throwing)).not.toThrow();
    expect(readStoredTheme(throwing)).toBeNull();
  });
});

describe("systemTheme", () => {
  it("按 prefers-color-scheme: dark 判定", () => {
    expect(systemTheme(fakeMedia(true))).toBe("dark");
    expect(systemTheme(fakeMedia(false))).toBe("light");
  });

  it("媒体查询不可用或抛异常时回退亮色", () => {
    expect(systemTheme(null)).toBe("light");
    expect(
      systemTheme(() => {
        throw new Error("matchMedia unavailable");
      }),
    ).toBe("light");
  });
});

describe("resolveInitialTheme", () => {
  it("无存储时跟随系统偏好", () => {
    expect(resolveInitialTheme(fakeStorage(null), fakeMedia(true))).toBe("dark");
    expect(resolveInitialTheme(fakeStorage(null), fakeMedia(false))).toBe("light");
  });

  it("存储值优先于系统偏好", () => {
    expect(resolveInitialTheme(fakeStorage("light"), fakeMedia(true))).toBe("light");
    expect(resolveInitialTheme(fakeStorage("dark"), fakeMedia(false))).toBe("dark");
  });

  it("非法存储值退回系统偏好", () => {
    expect(resolveInitialTheme(fakeStorage("sepia"), fakeMedia(true))).toBe("dark");
  });

  it("存储不可读时退回系统偏好", () => {
    const throwing = {
      getItem: () => {
        throw new Error("storage unavailable");
      },
    };
    expect(resolveInitialTheme(throwing, fakeMedia(true))).toBe("dark");
  });
});

describe("nextTheme", () => {
  it("双向切换", () => {
    expect(nextTheme("light")).toBe("dark");
    expect(nextTheme("dark")).toBe("light");
  });
});

describe("applyTheme", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("写入 document.documentElement.dataset.theme", () => {
    const dataset: Record<string, string> = {};
    vi.stubGlobal("document", { documentElement: { dataset } });

    applyTheme("dark");
    expect(dataset.theme).toBe("dark");

    applyTheme("light");
    expect(dataset.theme).toBe("light");
  });

  it("无 document（node 环境）时不抛异常", () => {
    expect(typeof document).toBe("undefined");
    expect(() => applyTheme("dark")).not.toThrow();
  });
});
