import { describe, expect, it } from "vitest";
import {
  activeSection,
  readSidebarCollapsed,
  SIDEBAR_COLLAPSED_KEY,
  SHELL_NAV_ITEMS,
  sectionLabel,
  writeSidebarCollapsed,
} from "./shellNav";

function memoryStorage(initial?: string): Pick<Storage, "getItem" | "setItem"> & {
  value: string | null;
} {
  const state = { value: initial ?? null };
  return {
    get value() {
      return state.value;
    },
    getItem(key: string) {
      return key === SIDEBAR_COLLAPSED_KEY ? state.value : null;
    },
    setItem(key: string, value: string) {
      if (key === SIDEBAR_COLLAPSED_KEY) {
        state.value = value;
      }
    },
  };
}

describe("activeSection", () => {
  it("把路由映射到导航分组", () => {
    expect(activeSection("/agents")).toBe("agents");
    expect(activeSection("/agents/builder")).toBe("agents");
    expect(activeSection("/harnesses")).toBe("harnesses");
    expect(activeSection("/rooms")).toBe("rooms");
    expect(activeSection("/rooms/1")).toBe("rooms");
    expect(activeSection("/rooms/1/runs/2")).toBe("rooms");
    expect(activeSection("/tasks")).toBe("tasks");
    expect(activeSection("/sessions")).toBe("sessions");
    expect(activeSection("/settings")).toBe("settings");
  });

  it("未知路径与根路径回退到智能体", () => {
    expect(activeSection("/")).toBe("agents");
    expect(activeSection("/unknown")).toBe("agents");
  });

  it("分组有可读标签", () => {
    expect(sectionLabel("agents")).toBe("智能体");
    expect(sectionLabel("harnesses")).toBe("Harness");
    expect(sectionLabel("rooms")).toBe("房间");
    expect(sectionLabel("tasks")).toBe("任务");
    expect(sectionLabel("sessions")).toBe("会话");
    expect(sectionLabel("settings")).toBe("设置");
  });

  it("会话排在任务之后", () => {
    const sections = SHELL_NAV_ITEMS.map((item) => item.section);
    expect(sections.indexOf("sessions")).toBe(sections.indexOf("tasks") + 1);
  });

  it("Harness 排在主导航第一位", () => {
    expect(SHELL_NAV_ITEMS[0]).toEqual({
      section: "harnesses",
      to: "/harnesses",
      label: "Harness",
    });
  });
});

describe("侧栏折叠存储", () => {
  it("读写往返保持布尔语义", () => {
    const storage = memoryStorage();
    expect(readSidebarCollapsed(storage)).toBe(false);

    writeSidebarCollapsed(true, storage);
    expect(storage.value).toBe("1");
    expect(readSidebarCollapsed(storage)).toBe(true);

    writeSidebarCollapsed(false, storage);
    expect(readSidebarCollapsed(storage)).toBe(false);
  });

  it("存储不可用或抛错时回退为展开", () => {
    const throwing = {
      getItem() {
        throw new Error("denied");
      },
      setItem() {
        throw new Error("denied");
      },
    };

    expect(readSidebarCollapsed(throwing)).toBe(false);
    expect(() => writeSidebarCollapsed(true, throwing)).not.toThrow();
  });

  it("非法存储值按展开处理", () => {
    expect(readSidebarCollapsed(memoryStorage("true"))).toBe(false);
    expect(readSidebarCollapsed(memoryStorage("collapsed"))).toBe(false);
    expect(readSidebarCollapsed(null)).toBe(false);
  });
});
