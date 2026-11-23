import { describe, expect, it } from "vitest";
import {
  computeMenuPosition,
  firstEnabledIndex,
  isNativeMenuTarget,
  nextEnabledIndex,
  type ContextMenuItem,
} from "./contextMenuCore";

function target(
  tagName: string,
  attributes: Record<string, string> = {},
  parentElement: unknown = null,
): EventTarget {
  return {
    tagName,
    getAttribute: (name: string) => attributes[name] ?? null,
    parentElement,
  } as unknown as EventTarget;
}

function item(id: string, disabled = false): ContextMenuItem {
  return { kind: "item", id, label: id, disabled, onSelect: () => {} };
}

const separator: ContextMenuItem = { kind: "separator", id: "sep" };

describe("computeMenuPosition", () => {
  const viewport = { width: 1200, height: 800 };
  const menuSize = { width: 200, height: 160 };

  it("默认从右键点向右下展开", () => {
    expect(computeMenuPosition({ x: 100, y: 100 }, menuSize, viewport)).toEqual({
      x: 100,
      y: 100,
    });
  });

  it("右侧放不下时翻转到锚点左侧", () => {
    expect(computeMenuPosition({ x: 1100, y: 100 }, menuSize, viewport)).toEqual({
      x: 900,
      y: 100,
    });
  });

  it("下方放不下时翻转到锚点上方", () => {
    expect(computeMenuPosition({ x: 100, y: 700 }, menuSize, viewport)).toEqual({
      x: 100,
      y: 540,
    });
  });

  it("右下同时越界时两轴一起翻转", () => {
    expect(computeMenuPosition({ x: 1150, y: 750 }, menuSize, viewport)).toEqual({
      x: 950,
      y: 590,
    });
  });

  it("翻转后仍越界时夹紧到边距", () => {
    expect(
      computeMenuPosition(
        { x: 200, y: 100 },
        menuSize,
        { width: 400, height: 800 },
      ),
    ).toEqual({ x: 8, y: 100 });
  });

  it("贴边打开时保留 8px 边距", () => {
    expect(computeMenuPosition({ x: 2, y: 2 }, menuSize, viewport)).toEqual({
      x: 8,
      y: 8,
    });
  });

  it("视口小于菜单时固定左上边距，由样式滚动兜底", () => {
    expect(
      computeMenuPosition(
        { x: 40, y: 40 },
        menuSize,
        { width: 150, height: 100 },
      ),
    ).toEqual({ x: 8, y: 8 });
  });
});

describe("firstEnabledIndex", () => {
  it("跳过分隔符与禁用项", () => {
    expect(firstEnabledIndex([separator, item("a", true), item("b")])).toBe(2);
  });

  it("没有可用项时返回 -1", () => {
    expect(firstEnabledIndex([separator, item("a", true)])).toBe(-1);
    expect(firstEnabledIndex([])).toBe(-1);
  });
});

describe("nextEnabledIndex", () => {
  const items = [item("a"), separator, item("disabled", true), item("b")];

  it("向下移动时跳过分隔符与禁用项", () => {
    expect(nextEnabledIndex(items, 0, 1)).toBe(3);
  });

  it("首尾循环", () => {
    expect(nextEnabledIndex(items, 3, 1)).toBe(0);
    expect(nextEnabledIndex(items, 0, -1)).toBe(3);
  });

  it("从 items.length 反向移动落到最后可用项", () => {
    expect(nextEnabledIndex(items, items.length, -1)).toBe(3);
  });

  it("全部不可用时返回 -1", () => {
    expect(nextEnabledIndex([separator, item("a", true)], 0, 1)).toBe(-1);
    expect(nextEnabledIndex([], 0, 1)).toBe(-1);
  });
});

describe("isNativeMenuTarget", () => {
  it("输入类元素保留原生菜单", () => {
    expect(isNativeMenuTarget(target("INPUT"))).toBe(true);
    expect(isNativeMenuTarget(target("TEXTAREA"))).toBe(true);
    expect(isNativeMenuTarget(target("SELECT"))).toBe(true);
  });

  it("contenteditable 自身与祖先都算可编辑", () => {
    expect(isNativeMenuTarget(target("DIV", { contenteditable: "" }))).toBe(
      true,
    );
    expect(
      isNativeMenuTarget(target("SPAN", {}, target("DIV", { contenteditable: "true" }))),
    ).toBe(true);
    expect(isNativeMenuTarget(target("DIV", { contenteditable: "false" }))).toBe(
      false,
    );
  });

  it("存在非空文本选区时保留原生菜单", () => {
    expect(isNativeMenuTarget(target("DIV"), "已选中的文字")).toBe(true);
    expect(isNativeMenuTarget(target("DIV"), "   ")).toBe(false);
  });

  it("普通目标不保留原生菜单", () => {
    expect(isNativeMenuTarget(target("DIV"))).toBe(false);
    expect(isNativeMenuTarget(null)).toBe(false);
  });
});
