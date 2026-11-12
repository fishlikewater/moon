import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { AcpCapabilityDetail, formatProbedAt } from "./acpCapabilityDetail";

const probed = {
  models: [
    { value: "default", name: "default" },
    { value: "opus", name: "opus" },
  ],
  modes: [
    { id: "default", name: "Default" },
    { id: "acceptEdits", name: "Accept Edits" },
  ],
  currentModelId: "opus",
  currentModeId: "acceptEdits",
  configOptions: [],
};

describe("AcpCapabilityDetail", () => {
  it("已探测：渲染条目名、返回按钮、两组 chips 与探测时间", () => {
    const html = renderToStaticMarkup(
      <AcpCapabilityDetail
        name="Claude ACP"
        capability={probed}
        probedAt={1_700_000_000_000}
        probeError={null}
        onBack={() => {}}
      />,
    );

    expect(html).toContain("Claude ACP");
    expect(html).toContain("返回列表");
    expect(html).toContain("capability-chip");
    expect(html).toContain("is-current");
    expect(html).toContain("当前");
    // 模型与权限模式各高亮当前项：任一映射失效时高亮数量会退化。
    expect((html.match(/capability-chip is-current/g) ?? []).length).toBe(2);
    expect(html).toMatch(
      /class="capability-chip is-current">Accept Edits<span class="capability-current-tag">当前<\/span>/,
    );
    expect(html).toContain("探测时间：");
    expect(html).not.toContain("尚未探测");
  });

  it("未探测：渲染占位文案且无 chips", () => {
    const html = renderToStaticMarkup(
      <AcpCapabilityDetail
        name="Smoke ACP"
        capability={null}
        probedAt={null}
        probeError={null}
        onBack={() => {}}
      />,
    );

    expect(html).toContain("尚未探测模型与权限模式");
    expect(html).not.toContain("capability-chip");
    expect(html).toContain("Smoke ACP");
  });

  it("探测错误以 role=alert 展示；无错误时不渲染错误区", () => {
    const withError = renderToStaticMarkup(
      <AcpCapabilityDetail
        name="X"
        capability={null}
        probedAt={null}
        probeError="ACP initialize 失败"
        onBack={() => {}}
      />,
    );
    expect(withError).toContain("role=\"alert\"");
    expect(withError).toContain("ACP initialize 失败");

    const withoutError = renderToStaticMarkup(
      <AcpCapabilityDetail
        name="X"
        capability={null}
        probedAt={null}
        probeError={null}
        onBack={() => {}}
      />,
    );
    expect(withoutError).not.toContain("role=\"alert\"");
  });

  it("formatProbedAt：null 为「未知」，时间戳为本地文本", () => {
    expect(formatProbedAt(null)).toBe("未知");
    expect(formatProbedAt(0)).not.toBe("未知");
  });
});
