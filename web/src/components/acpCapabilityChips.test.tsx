import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { CapabilityChips } from "./acpCapabilityChips";

describe("CapabilityChips", () => {
  it("多值时逐项渲染 chip，当前项带 is-current 与「当前」徽标", () => {
    const html = renderToStaticMarkup(
      <CapabilityChips
        label="模型"
        values={["default", "opus", "haiku"]}
        currentValue="opus"
      />,
    );

    expect((html.match(/class="capability-chip[" ]/g) ?? []).length).toBe(3);
    expect(html).toContain("is-current");
    expect(html).toContain(">opus<");
    expect(html).toContain("capability-current-tag");
    expect(html).toContain("当前");
    // 只有当前项出现一次徽标
    expect((html.match(/capability-current-tag/g) ?? []).length).toBe(1);
  });

  it("当前值不在宣告列表时追加一个当前 chip", () => {
    const html = renderToStaticMarkup(
      <CapabilityChips label="模型" values={["default"]} currentValue="opus" />,
    );

    expect((html.match(/class="capability-chip[" ]/g) ?? []).length).toBe(2);
    expect(html).toContain(">opus");
    expect((html.match(/capability-current-tag/g) ?? []).length).toBe(1);
  });

  it("空列表渲染「未宣告」且不产生 chips", () => {
    const html = renderToStaticMarkup(
      <CapabilityChips label="权限模式" values={[]} currentValue={null} />,
    );

    expect(html).toContain("capability-empty");
    expect(html).toContain("未宣告");
    expect(html).not.toContain('class="capability-chip');
    expect(html).toContain("权限模式");
  });

  it("currentValue 为 null 时全部 chip 无高亮无徽标", () => {
    const html = renderToStaticMarkup(
      <CapabilityChips
        label="模型"
        values={["default", "opus"]}
        currentValue={null}
      />,
    );

    expect(html).not.toContain("is-current");
    expect(html).not.toContain("capability-current-tag");
    expect((html.match(/class="capability-chip[" ]/g) ?? []).length).toBe(2);
  });
});
