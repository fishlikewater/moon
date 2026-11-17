import { describe, expect, it } from "vitest";
import type { AcpCapabilities } from "@moon/shared";
import {
  resolveAcpFormAvailability,
  unavailableHarnessWarning,
} from "./agentAcpForm";

const READY_CAPABILITIES: AcpCapabilities = {
  models: [
    { value: "m1", name: "M1" },
    { value: "m2", name: "M2" },
  ],
  currentModelId: null,
  modes: [
    { id: "default", name: "Default" },
    { id: "plan", name: "Plan" },
    { id: "auto", name: "Auto" },
  ],
  currentModeId: null,
  configOptions: [],
};

type Item = {
  id: number;
  name: string;
  capabilities: AcpCapabilities | null;
  probeError: string | null;
};

function item(
  id: number,
  overrides: Partial<Omit<Item, "id">> = {},
): Item {
  return {
    id,
    name: `agent-${id}`,
    capabilities: null,
    probeError: null,
    ...overrides,
  };
}

describe("resolveAcpFormAvailability", () => {
  it("加载中压过读取失败与已有列表", () => {
    expect(
      resolveAcpFormAvailability({
        loading: true,
        loadError: "接口 500",
        items: [item(1, { capabilities: READY_CAPABILITIES })],
        selectedId: "1",
      }),
    ).toEqual({ kind: "loading" });
  });

  it("读取失败压过空列表，避免把接口错误显示成注册表为空", () => {
    expect(
      resolveAcpFormAvailability({
        loading: false,
        loadError: "读取注册表失败：网络错误",
        items: [],
        selectedId: "",
      }),
    ).toEqual({ kind: "load-error", message: "读取注册表失败：网络错误" });
  });

  it("加载成功且无条目时才是空列表", () => {
    expect(
      resolveAcpFormAvailability({
        loading: false,
        loadError: null,
        items: [],
        selectedId: "",
      }),
    ).toEqual({ kind: "empty" });
  });

  it("未选择时统计可用/未检测/失败条目与首条失败原因", () => {
    expect(
      resolveAcpFormAvailability({
        loading: false,
        loadError: null,
        items: [
          item(1, { capabilities: READY_CAPABILITIES }),
          item(2, { capabilities: READY_CAPABILITIES }),
          item(3),
          item(4, { probeError: "找不到命令「npx」" }),
          item(5, { probeError: "未检测到 Node.js" }),
        ],
        selectedId: "",
      }),
    ).toEqual({
      kind: "unselected",
      usableCount: 2,
      pendingCount: 1,
      firstError: "找不到命令「npx」",
    });
  });

  it("全部未检测时 firstError 为 null", () => {
    expect(
      resolveAcpFormAvailability({
        loading: false,
        loadError: null,
        items: [item(1), item(2)],
        selectedId: "",
      }),
    ).toEqual({
      kind: "unselected",
      usableCount: 0,
      pendingCount: 2,
      firstError: null,
    });
  });

  it("全部失败时可用与未检测都为 0，仍保留失败原因", () => {
    expect(
      resolveAcpFormAvailability({
        loading: false,
        loadError: null,
        items: [item(1, { probeError: "未检测到 Node.js" })],
        selectedId: "",
      }),
    ).toEqual({
      kind: "unselected",
      usableCount: 0,
      pendingCount: 0,
      firstError: "未检测到 Node.js",
    });
  });

  it("选中项不在列表中时返回 selected-missing", () => {
    expect(
      resolveAcpFormAvailability({
        loading: false,
        loadError: null,
        items: [item(1)],
        selectedId: "9",
      }),
    ).toEqual({ kind: "selected-missing" });
  });

  it("选中项未检测时返回 selected-pending", () => {
    expect(
      resolveAcpFormAvailability({
        loading: false,
        loadError: null,
        items: [item(1, { name: "Kimi" })],
        selectedId: "1",
      }),
    ).toEqual({ kind: "selected-pending", name: "Kimi" });
  });

  it("选中项最近一次失败优先于旧的成功快照", () => {
    expect(
      resolveAcpFormAvailability({
        loading: false,
        loadError: null,
        items: [
          item(1, {
            name: "Kimi",
            capabilities: READY_CAPABILITIES,
            probeError: "未检测到 Node.js：该命令需要 Node.js 运行时",
          }),
        ],
        selectedId: "1",
      }),
    ).toEqual({
      kind: "selected-error",
      name: "Kimi",
      error: "未检测到 Node.js：该命令需要 Node.js 运行时",
    });
  });

  it("选中项可用时返回模型与模式数量", () => {
    expect(
      resolveAcpFormAvailability({
        loading: false,
        loadError: null,
        items: [
          item(1, {
            name: "Kimi",
            capabilities: READY_CAPABILITIES,
          }),
        ],
        selectedId: "1",
      }),
    ).toEqual({
      kind: "selected-ready",
      name: "Kimi",
      modelCount: 2,
      modeCount: 3,
    });
  });
});

describe("unavailableHarnessWarning", () => {
  it("选中失败项时给出原因与修复建议", () => {
    const warning = unavailableHarnessWarning({
      kind: "selected-error",
      name: "Kimi",
      error: "找不到命令「kimi」",
    });

    expect(warning).toContain("「Kimi」当前不可用：找不到命令「kimi」");
    expect(warning).toContain("仍要保存吗");
    expect(warning).toContain("Harness 页面修复");
  });

  it("选中未检测项时提示先检测", () => {
    const warning = unavailableHarnessWarning({
      kind: "selected-pending",
      name: "Codex",
    });

    expect(warning).toContain("「Codex」尚未检测能力");
    expect(warning).toContain("仍要保存吗");
    expect(warning).toContain("立即检测");
  });

  it("可用或其他分支不需要保存确认", () => {
    expect(
      unavailableHarnessWarning({
        kind: "selected-ready",
        name: "Kimi",
        modelCount: 1,
        modeCount: 1,
      }),
    ).toBeNull();
    expect(
      unavailableHarnessWarning({
        kind: "unselected",
        usableCount: 0,
        pendingCount: 0,
        firstError: null,
      }),
    ).toBeNull();
    expect(unavailableHarnessWarning({ kind: "empty" })).toBeNull();
  });
});
