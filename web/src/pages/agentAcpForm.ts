import type { CustomHarness } from "@moon/shared";
import { resolveAcpAvailability } from "../components/AcpHarnessRegistry";

/** 表单判定所需的条目字段；与注册表列表同源，缺字段时直接报错而不是猜测。 */
type AcpFormHarness = Pick<
  CustomHarness,
  "id" | "name" | "capabilities" | "probeError"
>;

/** 智能体表单内 ACP 接入的可用性状态；分支与引导文案一一对应。 */
export type AcpFormAvailability =
  | { kind: "loading" }
  | { kind: "load-error"; message: string }
  | { kind: "empty" }
  | {
      kind: "unselected";
      usableCount: number;
      pendingCount: number;
      /** 第一条失败原因；可用或全部未检测时为 null。 */
      firstError: string | null;
    }
  | { kind: "selected-missing" }
  | { kind: "selected-pending"; name: string }
  | { kind: "selected-error"; name: string; error: string }
  | {
      kind: "selected-ready";
      name: string;
      modelCount: number;
      modeCount: number;
    };

/**
 * 纯函数：按注册表加载状态与当前选择推导表单引导分支。
 *
 * 优先级：加载中 > 读取失败 > 空列表 > 选中项 > 未选择。加载失败必须压过
 * 「空」，否则接口报错会伪装成「没有注册 ACP agent」。选中项优先级高于
 * 未选择统计，选中条目的状态才是用户下一步要看的内容。
 */
export function resolveAcpFormAvailability(input: {
  loading: boolean;
  loadError: string | null;
  items: readonly AcpFormHarness[];
  /** 表单里的 customHarnessId；空字符串表示未选择。 */
  selectedId: string;
}): AcpFormAvailability {
  if (input.loading) {
    return { kind: "loading" };
  }
  if (input.loadError !== null) {
    return { kind: "load-error", message: input.loadError };
  }
  if (input.items.length === 0) {
    return { kind: "empty" };
  }

  if (input.selectedId !== "") {
    const selected = input.items.find(
      (item) => String(item.id) === input.selectedId,
    );
    if (selected === undefined) {
      return { kind: "selected-missing" };
    }
    // 与注册表徽标共用同一优先级：最近一次失败优先于旧的成功快照。
    const tone = resolveAcpAvailability(selected).tone;
    if (tone === "error") {
      return {
        kind: "selected-error",
        name: selected.name,
        error: selected.probeError ?? "未知原因",
      };
    }
    if (tone === "pending") {
      return { kind: "selected-pending", name: selected.name };
    }
    return {
      kind: "selected-ready",
      name: selected.name,
      modelCount: selected.capabilities?.models.length ?? 0,
      modeCount: selected.capabilities?.modes.length ?? 0,
    };
  }

  let usableCount = 0;
  let pendingCount = 0;
  let firstError: string | null = null;
  for (const item of input.items) {
    const tone = resolveAcpAvailability(item).tone;
    if (tone === "ready") {
      usableCount += 1;
    } else if (tone === "pending") {
      pendingCount += 1;
    } else if (firstError === null) {
      firstError = item.probeError;
    }
  }
  return { kind: "unselected", usableCount, pendingCount, firstError };
}

/**
 * 纯函数：保存未就绪 ACP agent 前的确认文案；可用或其他分支返回 null。
 *
 * 表单允许先保存草稿，但必须让用户知道当前选择运行会失败，并给出下一步。
 */
export function unavailableHarnessWarning(
  availability: AcpFormAvailability,
): string | null {
  if (availability.kind === "selected-error") {
    return `「${availability.name}」当前不可用：${availability.error}\n仍要保存吗？保存后运行会失败，建议先在 Harness 页面修复并重新检测。`;
  }
  if (availability.kind === "selected-pending") {
    return `「${availability.name}」尚未检测能力，运行可能失败。\n仍要保存吗？建议先点「立即检测」。`;
  }
  return null;
}
