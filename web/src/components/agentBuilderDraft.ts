import type { CustomHarness, HarnessModelOption } from "@moon/shared";
import { acpReasoningOption } from "../acpAgentCapabilities";

/**
 * ACP 没有全局模型目录：草稿模型候选来自所选注册表条目最近一次探测结果。
 * 与新建智能体弹层保持同一映射规则。
 */
export function acpDraftModels(
  customHarness: CustomHarness | undefined,
): HarnessModelOption[] {
  if (customHarness?.capabilities == null) {
    return [];
  }
  return customHarness.capabilities.models.map((model) => ({
    key: `acp:${customHarness.id}:${model.value}`,
    providerId: "acp",
    providerName: customHarness.name,
    modelId: model.value,
    reasoningLevels: [],
    defaultReasoningLevel: null,
  }));
}

/** ACP 思考等级候选：来自所选注册表条目宣告的思考类 configOption。 */
export function acpDraftReasoningLevels(
  customHarness: CustomHarness | undefined,
): string[] {
  return (
    acpReasoningOption(customHarness?.capabilities ?? null) ?? []
  ).map((choice) => choice.value);
}
