import type { HarnessKind, HarnessModelCatalog } from "@moon/shared";
import { describe, expect, it } from "vitest";
import {
  BUILTIN_HARNESSES,
  HARNESS_STATUS_LABELS,
  catalogStatusText,
  type BuiltinHarnessInfo,
} from "./harnessCatalog";

function harness(kind: HarnessKind): BuiltinHarnessInfo {
  const found = BUILTIN_HARNESSES.find((item) => item.kind === kind);
  if (found === undefined) {
    throw new Error(`缺少 harness 元数据：${kind}`);
  }
  return found;
}

function catalog(
  harnessKind: HarnessKind,
  overrides: Partial<HarnessModelCatalog> = {},
): HarnessModelCatalog {
  return {
    harness: harnessKind,
    available: true,
    models: [],
    message: null,
    ...overrides,
  };
}

describe("内置 harness 元数据", () => {
  it("覆盖全部 HarnessKind 且标签唯一", () => {
    expect(BUILTIN_HARNESSES.map((item) => item.kind)).toEqual([
      "zcode",
      "codex",
      "kimi-code",
      "acp",
      "fake",
    ]);
    expect(new Set(BUILTIN_HARNESSES.map((item) => item.label)).size).toBe(
      BUILTIN_HARNESSES.length,
    );
    expect(BUILTIN_HARNESSES.map((item) => item.status)).toEqual([
      "runnable",
      "runnable",
      "placeholder",
      "runnable",
      "test-only",
    ]);
  });

  it("状态都有可读标签", () => {
    expect(HARNESS_STATUS_LABELS.runnable).toBe("可运行");
    expect(HARNESS_STATUS_LABELS.placeholder).toBe("占位");
    expect(HARNESS_STATUS_LABELS["test-only"]).toBe("仅测试");
  });
});

describe("catalogStatusText", () => {
  it("加载中、可用、不可用与失败分别给出可读说明", () => {
    const zcode = harness("zcode");
    expect(catalogStatusText(zcode, { status: "loading" })).toBe(
      "正在读取模型目录…",
    );
    expect(
      catalogStatusText(zcode, {
        status: "ready",
        catalog: catalog("zcode", { models: [] }),
      }),
    ).toBe("模型目录可用：0 个模型");
    expect(
      catalogStatusText(zcode, {
        status: "ready",
        catalog: catalog("zcode", {
          available: false,
          message: "未找到 ZCode 配置",
        }),
      }),
    ).toBe("模型目录不可用：未找到 ZCode 配置");
    expect(
      catalogStatusText(zcode, { status: "error", message: "offline" }),
    ).toBe("模型目录读取失败：offline");
  });

  it("ACP 与 fake 不显示模型数量", () => {
    expect(catalogStatusText(harness("acp"), undefined)).toBe(
      "模型由 ACP agent 探测决定",
    );
    expect(catalogStatusText(harness("fake"), undefined)).toBe("不使用模型");
  });
});
