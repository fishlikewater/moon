import { afterEach, beforeEach, describe, expect, it } from "vitest";
import {
  nextRoomSideTab,
  parseRoomSideTab,
  persistRoomSideTab,
  readRoomSideTab,
} from "./roomSidebarTabs";

const originalWindow = (globalThis as { window?: unknown }).window;

interface FakeStorage {
  value: string | null;
  failGet: boolean;
  failSet: boolean;
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

function installWindow(storage: FakeStorage): void {
  Object.defineProperty(globalThis, "window", {
    value: { localStorage: storage },
    configurable: true,
    writable: true,
  });
}

function fakeStorage(initial: string | null = null): FakeStorage {
  return {
    value: initial,
    failGet: false,
    failSet: false,
    getItem() {
      if (this.failGet) throw new Error("storage unavailable");
      return this.value;
    },
    setItem(_key: string, value: string) {
      if (this.failSet) throw new Error("storage unavailable");
      this.value = value;
    },
  };
}

beforeEach(() => {
  installWindow(fakeStorage());
});

afterEach(() => {
  Object.defineProperty(globalThis, "window", {
    value: originalWindow,
    configurable: true,
    writable: true,
  });
});

describe("roomSidebarTabs", () => {
  it("只接受已知标签 id，其余回退概览", () => {
    expect(parseRoomSideTab("overview")).toBe("overview");
    expect(parseRoomSideTab("files")).toBe("files");
    expect(parseRoomSideTab("members")).toBe("members");
    expect(parseRoomSideTab("queue")).toBe("queue");
    expect(parseRoomSideTab(null)).toBe("overview");
    expect(parseRoomSideTab(undefined)).toBe("overview");
    expect(parseRoomSideTab("")).toBe("overview");
    expect(parseRoomSideTab("files ")).toBe("overview");
    expect(parseRoomSideTab("bogus")).toBe("overview");
  });

  it("左右方向键循环，Home/End 到两端，其他键不处理", () => {
    expect(nextRoomSideTab("overview", "ArrowRight")).toBe("files");
    expect(nextRoomSideTab("files", "ArrowRight")).toBe("members");
    expect(nextRoomSideTab("queue", "ArrowRight")).toBe("overview");
    expect(nextRoomSideTab("overview", "ArrowLeft")).toBe("queue");
    expect(nextRoomSideTab("queue", "ArrowLeft")).toBe("members");
    expect(nextRoomSideTab("files", "Home")).toBe("overview");
    expect(nextRoomSideTab("files", "End")).toBe("queue");
    expect(nextRoomSideTab("files", "Tab")).toBeNull();
    expect(nextRoomSideTab("files", "Enter")).toBeNull();
  });

  it("读写 localStorage，存储异常时读回概览且写入不抛出", () => {
    const storage = fakeStorage();
    installWindow(storage);

    expect(readRoomSideTab()).toBe("overview");
    persistRoomSideTab("queue");
    expect(storage.value).toBe("queue");
    expect(readRoomSideTab()).toBe("queue");

    storage.failGet = true;
    expect(readRoomSideTab()).toBe("overview");
    storage.failGet = false;

    storage.value = "bogus";
    expect(readRoomSideTab()).toBe("overview");

    storage.failSet = true;
    expect(() => persistRoomSideTab("files")).not.toThrow();
  });
});
