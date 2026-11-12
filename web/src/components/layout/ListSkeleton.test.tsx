import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { ListSkeleton } from "./ListSkeleton";

function rowCount(html: string): number {
  return html.split("skeleton-row").length - 1;
}

describe("ListSkeleton", () => {
  it("按变体渲染并暴露加载状态", () => {
    const html = renderToStaticMarkup(
      <ListSkeleton variant="agent" count={3} />,
    );

    expect(html).toContain('role="status"');
    expect(html).toContain('aria-label="正在加载列表"');
    expect(html).toContain("skeleton-agent");
    expect(rowCount(html)).toBe(3);
  });

  it("默认渲染 6 行且任务变体使用独立类名", () => {
    const html = renderToStaticMarkup(<ListSkeleton variant="task" />);

    expect(html).toContain("skeleton-task");
    expect(rowCount(html)).toBe(6);
  });

  it("非法数量至少保留一行", () => {
    const html = renderToStaticMarkup(
      <ListSkeleton variant="room" count={0} />,
    );

    expect(rowCount(html)).toBe(1);
  });
});
