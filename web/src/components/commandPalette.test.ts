import { describe, expect, it } from "vitest";
import {
  buildCommandItems,
  filterCommands,
  nextIndex,
  type CommandEntities,
} from "./commandPalette";

const ENTITIES: CommandEntities = {
  rooms: [
    { id: 1, name: "设计讨论" },
    { id: 2, name: "发布筹备" },
  ],
  agents: [
    { id: 10, name: "小月" },
    { id: 11, name: "阿蓝" },
  ],
  tasks: [{ id: 100, title: "整理验收清单", roomId: 1 }],
};

describe("buildCommandItems", () => {
  it("包含动作、页面、房间、任务与智能体五组", () => {
    const items = buildCommandItems(ENTITIES);
    const groups = new Set(items.map((item) => item.group));
    expect([...groups].sort()).toEqual(
      ["动作", "页面", "房间", "任务", "智能体"].sort(),
    );
    expect(items.find((item) => item.id === "action-new-room")?.action).toBe(
      "new-room",
    );
    expect(items.find((item) => item.id === "room-2")?.to).toBe("/rooms/2");
  });
});

describe("filterCommands", () => {
  it("空查询先显示动作与页面", () => {
    const result = filterCommands(buildCommandItems(ENTITIES), "");
    expect(result.slice(0, 2).every((item) => item.group === "动作")).toBe(true);
    expect(result.some((item) => item.group === "页面")).toBe(true);
  });

  it("按标签与关键字过滤，多关键词全部命中", () => {
    const items = buildCommandItems(ENTITIES);
    expect(filterCommands(items, "设计")[0]?.label).toBe("设计讨论");
    expect(filterCommands(items, "room 发布")[0]?.label).toBe("发布筹备");
    expect(filterCommands(items, "任务 验收")[0]?.label).toBe("整理验收清单");
    expect(filterCommands(items, "不存在的词")).toEqual([]);
  });

  it("前缀匹配优先于包含匹配", () => {
    const items = [
      {
        id: "contains",
        label: "打开设置",
        group: "动作" as const,
        keywords: "",
      },
      {
        id: "prefix",
        label: "设置",
        group: "页面" as const,
        keywords: "",
      },
    ];
    expect(filterCommands(items, "设置")[0]?.id).toBe("prefix");
  });

  it("按 limit 截断", () => {
    expect(filterCommands(buildCommandItems(ENTITIES), "", 3).length).toBe(3);
  });
});

describe("nextIndex", () => {
  it("循环移动且空列表回 0", () => {
    expect(nextIndex(0, 3, 1)).toBe(1);
    expect(nextIndex(2, 3, 1)).toBe(0);
    expect(nextIndex(0, 3, -1)).toBe(2);
    expect(nextIndex(0, 0, 1)).toBe(0);
  });
});
