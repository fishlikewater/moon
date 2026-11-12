import type { HarnessModelCatalog, HarnessModelOption } from "@moon/shared";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { HarnessOverview } from "./HarnessOverview";

function model(index: number): HarnessModelOption {
  return {
    key: `provider/model-${index}`,
    providerId: "provider",
    providerName: "Provider",
    modelId: `model-${index}`,
    reasoningLevels: [],
    defaultReasoningLevel: null,
  };
}

const readyCatalog: HarnessModelCatalog = {
  harness: "zcode",
  available: true,
  models: [model(1), model(2)],
  message: null,
};

describe("HarnessOverview", () => {
  it("渲染五种内置 harness、状态徽标与各自的目录说明", () => {
    const html = renderToStaticMarkup(
      <HarnessOverview
        acpCount={2}
        states={{
          zcode: { status: "ready", catalog: readyCatalog },
          codex: { status: "error", message: "offline" },
          "kimi-code": {
            status: "ready",
            catalog: {
              harness: "kimi-code",
              available: false,
              models: [],
              message: "未找到 Kimi Code 配置",
            },
          },
        }}
      />,
    );

    expect(html).toContain("内置 harness");
    expect(html).toContain("ZCode");
    expect(html).toContain("Codex");
    expect(html).toContain("Kimi Code");
    expect(html).toContain("ACP");
    expect(html).toContain("Fake");
    expect(html).toContain("可运行");
    expect(html).toContain("占位");
    expect(html).toContain("仅测试");
    expect(html).toContain("模型目录可用：2 个模型");
    expect(html).toContain("模型目录读取失败：offline");
    expect(html).toContain("模型目录不可用：未找到 Kimi Code 配置");
    expect(html).toContain("注册表 2 个 agent");
    expect(html).toContain("不使用模型");
  });

  it("目录尚未返回时显示读取中，不伪造模型数量", () => {
    const html = renderToStaticMarkup(
      <HarnessOverview acpCount={0} states={{}} />,
    );

    expect(html).toContain("正在读取模型目录…");
    expect(html).not.toContain("模型目录可用");
  });
});
