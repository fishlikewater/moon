import { describe, expect, it } from "vitest";
import {
  FIRST_PROBE_TIMEOUT_SECS,
  acpUsableFromItems,
  formatRelativeTime,
  nextAutoProbeTarget,
  probeTimeoutFor,
  resolveAcpAvailability,
  shouldAutoProbe,
} from "./AcpHarnessRegistry";
import { shouldShowOnboarding } from "./AcpOnboardingGuide";

const READY_CAPABILITIES = {
  models: [
    { value: "m1", name: "M1" },
    { value: "m2", name: "M2" },
  ],
  currentModelId: null,
  modes: [{ id: "default", name: "Default" }],
  currentModeId: null,
  configOptions: [],
};

describe("resolveAcpAvailability", () => {
  it("探测成功显示模型与模式数量", () => {
    expect(
      resolveAcpAvailability({
        capabilities: READY_CAPABILITIES,
        probeError: null,
      }),
    ).toEqual({
      tone: "ready",
      label: "已探测 · 2 模型 1 模式",
    });
  });

  it("最近一次失败优先于旧的成功快照（卸载 Node 场景）", () => {
    expect(
      resolveAcpAvailability({
        capabilities: READY_CAPABILITIES,
        probeError: "未检测到 Node.js：该命令需要 Node.js 运行时",
      }),
    ).toEqual({ tone: "error", label: "不可用" });
  });

  it("失败显示不可用，未探测显示未检测", () => {
    expect(
      resolveAcpAvailability({
        capabilities: null,
        probeError: "找不到命令「npx」：请确认已安装并加入 PATH",
      }),
    ).toEqual({ tone: "error", label: "不可用" });
    expect(
      resolveAcpAvailability({ capabilities: null, probeError: null }),
    ).toEqual({ tone: "pending", label: "未检测" });
  });
});

describe("shouldAutoProbe", () => {
  it("成功或失败过的条目不再自动探测", () => {
    expect(shouldAutoProbe({ capabilities: null, probeError: null })).toBe(
      true,
    );
    expect(
      shouldAutoProbe({
        capabilities: null,
        probeError: "未检测到 Node.js",
      }),
    ).toBe(false);
    expect(
      shouldAutoProbe({
        capabilities: READY_CAPABILITIES,
        probeError: null,
      }),
    ).toBe(false);
  });
});

describe("nextAutoProbeTarget", () => {
  const items = [
    { id: 1, capabilities: null, probeError: null },
    { id: 2, capabilities: null, probeError: "失败" },
    { id: 3, capabilities: null, probeError: null },
    { id: 4, capabilities: READY_CAPABILITIES, probeError: null },
  ] as const;

  it("按列表顺序取第一个从未探测的条目", () => {
    expect(nextAutoProbeTarget(items, new Set())?.id).toBe(1);
    expect(nextAutoProbeTarget(items, new Set([1]))?.id).toBe(3);
  });

  it("已探测过（成功或失败）与已排队的条目不再入选", () => {
    expect(nextAutoProbeTarget(items, new Set([1, 3]))).toBeNull();
  });
});

describe("probeTimeoutFor", () => {
  it("从未探测条目放宽到 90 秒，其余走服务端默认", () => {
    expect(probeTimeoutFor({ capabilities: null, probeError: null })).toBe(90);
    expect(
      probeTimeoutFor({
        capabilities: null,
        probeError: "失败",
      }),
    ).toBeUndefined();
    expect(
      probeTimeoutFor({ capabilities: READY_CAPABILITIES, probeError: null }),
    ).toBeUndefined();
    expect(FIRST_PROBE_TIMEOUT_SECS).toBe(90);
  });
});

describe("acpUsableFromItems", () => {
  it("最近一次失败优先：旧成功快照加新失败不算可用", () => {
    expect(
      acpUsableFromItems([
        { capabilities: READY_CAPABILITIES, probeError: "未检测到 Node.js" },
      ]),
    ).toBe(false);
    expect(
      acpUsableFromItems([{ capabilities: READY_CAPABILITIES, probeError: null }]),
    ).toBe(true);
    expect(acpUsableFromItems([])).toBe(false);
  });
});

describe("formatRelativeTime", () => {
  const now = 1_700_000_000_000;
  const minute = 60_000;
  const hour = 3_600_000;

  it("一分钟内显示刚刚，一小时内显示分钟，一天内显示小时", () => {
    expect(formatRelativeTime(now - 30_000, now)).toBe("刚刚");
    expect(formatRelativeTime(now - 5 * minute, now)).toBe("5 分钟前");
    expect(formatRelativeTime(now - 3 * hour, now)).toBe("3 小时前");
  });

  it("超过一天回退为绝对时间", () => {
    const text = formatRelativeTime(now - 2 * 86_400_000, now);
    expect(text).not.toBe("刚刚");
    expect(text).not.toContain("前");
  });
});

describe("shouldShowOnboarding", () => {
  it("数据未就绪、任一接入可用时不显示引导", () => {
    expect(
      shouldShowOnboarding({ settled: false, internalUsable: false, acpUsable: false }),
    ).toBe(false);
    expect(
      shouldShowOnboarding({ settled: true, internalUsable: true, acpUsable: false }),
    ).toBe(false);
    expect(
      shouldShowOnboarding({ settled: true, internalUsable: false, acpUsable: true }),
    ).toBe(false);
  });

  it("两份数据就绪且全部不可用时显示引导", () => {
    expect(
      shouldShowOnboarding({ settled: true, internalUsable: false, acpUsable: false }),
    ).toBe(true);
  });
});
