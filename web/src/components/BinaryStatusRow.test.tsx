import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { BinaryStatusRow } from "./BinaryStatusRow";

describe("BinaryStatusRow", () => {
  it("未检测时显示占位而不是状态徽标", () => {
    const html = renderToStaticMarkup(
      <BinaryStatusRow name="zcode" status={undefined} />,
    );
    expect(html).toContain("尚未检测");
    expect(html).not.toContain("badge-ready");
  });

  it("可用时显示状态、来源与路径", () => {
    const html = renderToStaticMarkup(
      <BinaryStatusRow
        name="zcode"
        status={{
          path: String.raw`D:\ZCode\resources\glm\zcode.cjs`,
          source: "protocol",
          exists: true,
          searched: [],
        }}
      />,
    );
    expect(html).toContain("可用");
    expect(html).toContain("URL 协议处理器");
    expect(html).toContain(String.raw`D:\ZCode\resources\glm\zcode.cjs`);
    expect(html).not.toContain("已尝试");
  });

  it("路径不存在时显示警告徽标与已尝试来源", () => {
    const html = renderToStaticMarkup(
      <BinaryStatusRow
        name="codex"
        status={{
          path: "codex",
          source: "fallback",
          exists: false,
          searched: ["app_paths:codex.exe", "path:codex"],
        }}
      />,
    );
    expect(html).toContain("未检测到");
    expect(html).toContain("badge-warn");
    expect(html).toContain("已尝试：app_paths:codex.exe、path:codex");
  });
});
