import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import MarkdownContent from "./MarkdownContent";

describe("MarkdownContent", () => {
  it("渲染标题、表格与代码块", () => {
    const html = renderToStaticMarkup(
      <MarkdownContent
        text={[
          "# 结论",
          "",
          "| 项 | 值 |",
          "| --- | --- |",
          "| A | 1 |",
          "",
          "```ts",
          "const value = 1;",
          "```",
        ].join("\n")}
      />,
    );
    expect(html).toContain("<h1>结论</h1>");
    expect(html).toContain("<table>");
    expect(html).toContain("<code");
  });

  it("不执行原始 HTML 脚本", () => {
    const html = renderToStaticMarkup(
      <MarkdownContent text={"正常文本\n\n<script>alert(1)</script>"} />,
    );
    expect(html).not.toContain("<script>");
    expect(html).not.toContain("alert(1)");
    expect(html).toContain("正常文本");
  });

  it("删除远程图片，避免外部请求与跟踪像素", () => {
    const html = renderToStaticMarkup(
      <MarkdownContent text={"![跟踪像素](https://example.com/pixel.png)"} />,
    );
    expect(html).not.toContain("<img");
    expect(html).not.toContain("pixel.png");
  });

  it("剥离 javascript 与 data 协议的链接", () => {
    const html = renderToStaticMarkup(
      <MarkdownContent
        text={"[危险](javascript:alert(1))\n\n[数据](data:text/html,boom)"}
      />,
    );
    expect(html).not.toContain("javascript:");
    expect(html).not.toContain("data:text/html");
  });

  it("链接在新标签打开并带安全属性", () => {
    const html = renderToStaticMarkup(
      <MarkdownContent text={"[示例](https://example.com)"} />,
    );
    expect(html).toContain('href="https://example.com"');
    expect(html).toContain('target="_blank"');
    expect(html).toContain('rel="noopener noreferrer"');
  });
});
