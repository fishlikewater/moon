import { type ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it } from "vitest";
import { AcpFormGuide } from "./AcpFormGuide";

function renderGuide(html: ReactElement): string {
  return renderToStaticMarkup(<MemoryRouter>{html}</MemoryRouter>);
}

describe("AcpFormGuide", () => {
  it("失败态给出原因、排查顺序、重新检测与修复入口", () => {
    const html = renderGuide(
      <AcpFormGuide
        availability={{
          kind: "selected-error",
          name: "Kimi Code",
          error: "找不到命令「kimi」：请确认已安装并加入 PATH",
        }}
        onReprobe={() => {}}
      />,
    );

    expect(html).toContain("「Kimi Code」不可用");
    expect(html).toContain("找不到命令「kimi」");
    expect(html).toContain("检查顺序：命令与 PATH → Node.js 运行时");
    expect(html).toContain("重新检测");
    expect(html).toContain("打开 Harness 页面修复");
    expect(html).toContain('href="/harnesses"');
  });

  it("未检测态提供立即检测", () => {
    const html = renderGuide(
      <AcpFormGuide
        availability={{ kind: "selected-pending", name: "Codex ACP" }}
        onReprobe={() => {}}
      />,
    );

    expect(html).toContain("尚未检测能力");
    expect(html).toContain("立即检测");
    expect(html).not.toContain("检查顺序");
  });

  it("检测中禁用按钮并显示进行态", () => {
    const html = renderGuide(
      <AcpFormGuide
        availability={{ kind: "selected-pending", name: "Codex ACP" }}
        onReprobe={() => {}}
        reprobeing
      />,
    );

    expect(html).toContain("检测中…");
    expect(html).toMatch(/<button[^>]*disabled=""[^>]*>检测中…<\/button>/);
  });

  it("可用态只展示能力数量，不渲染动作", () => {
    const html = renderGuide(
      <AcpFormGuide
        availability={{
          kind: "selected-ready",
          name: "Kimi Code",
          modelCount: 2,
          modeCount: 3,
        }}
        onReprobe={() => {}}
      />,
    );

    expect(html).toContain("2 个模型、3 个会话模式");
    expect(html).not.toContain("重新检测");
    expect(html).not.toContain("href=\"/harnesses\"");
  });
});
