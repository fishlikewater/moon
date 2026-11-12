import { afterEach, describe, expect, it, vi } from "vitest";
import { isDesktopShell } from "./desktopShell";

/**
 * 桌面壳判定行为（node 环境，不依赖 jsdom）。
 *
 * 窗口控制按钮只在 Tauri 初始化脚本注入 `__MOON_API_BASE__` 后渲染，浏览器
 * 开发模式必须判定为非桌面壳。这里用受控 fake window 钉住「必须是字符串」
 * 这一契约：只判断 window 是否存在、或只判断值是否真值，都会在此失败。
 */
describe("isDesktopShell", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("没有 window 全局时返回 false", () => {
    vi.stubGlobal("window", undefined);
    expect(isDesktopShell()).toBe(false);
  });

  it("window.__MOON_API_BASE__ 为字符串时返回 true", () => {
    vi.stubGlobal("window", { __MOON_API_BASE__: "http://127.0.0.1:3000" });
    expect(isDesktopShell()).toBe(true);
  });

  it("__MOON_API_BASE__ 为 undefined 时返回 false", () => {
    vi.stubGlobal("window", { __MOON_API_BASE__: undefined });
    expect(isDesktopShell()).toBe(false);
  });

  it("__MOON_API_BASE__ 为数字时返回 false", () => {
    vi.stubGlobal("window", { __MOON_API_BASE__: 3000 });
    expect(isDesktopShell()).toBe(false);
  });

  it("__MOON_API_BASE__ 为对象时返回 false", () => {
    vi.stubGlobal("window", { __MOON_API_BASE__: {} });
    expect(isDesktopShell()).toBe(false);
  });
});
