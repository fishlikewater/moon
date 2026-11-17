//! agent 表单的供应商联动纯函数。
//!
//! 三级联动：供应商（仅启用）→ 模型（供应商 models + 手填）→ 思考等级
//! （所选模型的 reasoningLevels）；切换供应商清空下游选择。

import type { ApiProvider, ProviderModel } from "@moon/shared";

/** 表单下拉候选：仅启用的供应商（停用条目仍可能被旧绑定引用）。 */
export function resolveProviderOptions(items: ApiProvider[]): ApiProvider[] {
  return items.filter((item) => item.enabled);
}

/** 按表单值查供应商条目（表单存字符串 id）。 */
export function findProviderById(
  items: ApiProvider[],
  providerId: string,
): ApiProvider | undefined {
  if (providerId === "") {
    return undefined;
  }
  return items.find((item) => String(item.id) === providerId);
}

/** 供应商的模型候选；未绑定供应商时为空（回退到 ACP 探测候选）。 */
export function providerModelChoices(
  provider: ApiProvider | undefined,
): ProviderModel[] {
  return provider?.models ?? [];
}

/** 所选供应商模型的思考等级候选；模型未声明时为空。 */
export function providerReasoningChoices(
  provider: ApiProvider | undefined,
  modelId: string,
): string[] {
  if (modelId === "") {
    return [];
  }
  return (
    provider?.models.find((model) => model.id === modelId)?.reasoningLevels ??
    []
  );
}

/** 切换供应商后清空下游：模型与思考等级都随候选变化失效。 */
export function nextProviderSelection(providerId: string): {
  apiProviderId: string;
  model: string;
  reasoningLevel: string;
} {
  return { apiProviderId: providerId, model: "", reasoningLevel: "" };
}

/**
 * 切换自定义 ACP agent 后清空下游选择。
 *
 * 模型、思考等级与会话模式都来自上一个 agent 的探测快照；保留旧值会让
 * 运行时命中不了新 agent 的宣告值，退化成「未宣告」警告。供应商绑定
 * 独立于具体 agent，由调用方展开当前表单保留。
 */
export function nextCustomHarnessSelection(value: string): {
  customHarnessId: string;
  model: string;
  reasoningLevel: string;
  sessionMode: string;
} {
  return {
    customHarnessId: value,
    model: "",
    reasoningLevel: "",
    sessionMode: "",
  };
}
