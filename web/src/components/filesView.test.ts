import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { fileDownloadUrl } from "./filesView";

const originalWindow = (globalThis as { window?: unknown }).window;

function setApiBase(base: string): void {
  (globalThis as { window: { __MOON_API_BASE__?: string } }).window.__MOON_API_BASE__ =
    base;
}

beforeEach(() => {
  Object.defineProperty(globalThis, "window", {
    value: { __MOON_API_BASE__: "" },
    configurable: true,
    writable: true,
  });
});

afterEach(() => {
  Object.defineProperty(globalThis, "window", {
    value: originalWindow,
    configurable: true,
    writable: true,
  });
});

describe("filesView", () => {
  it("浏览器模式返回相对下载地址并编码参数", () => {
    expect(fileDownloadUrl(3, "agent-7", "notes/会议 记录.md")).toBe(
      "/api/rooms/3/files/download?root=agent-7&path=notes%2F%E4%BC%9A%E8%AE%AE%20%E8%AE%B0%E5%BD%95.md",
    );
    expect(fileDownloadUrl(1, "room-0", "a&b#c.txt")).toBe(
      "/api/rooms/1/files/download?root=room-0&path=a%26b%23c.txt",
    );
  });

  it("桌面端注入 base 后返回指向本地 API 的绝对地址", () => {
    setApiBase("http://127.0.0.1:39001/");
    expect(fileDownloadUrl(3, "room-0", "agent-note.txt")).toBe(
      "http://127.0.0.1:39001/api/rooms/3/files/download?root=room-0&path=agent-note.txt",
    );
  });
});
