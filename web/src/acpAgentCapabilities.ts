/**
 * ACP agent 能力的表单侧纯函数：会话模式与思考等级的候选推导。
 *
 * 与后端 `acp.rs::reasoning_config_option` 使用同一识别规则：
 * 思考类 configOption = category 或 id 不区分大小写含 think/reason。
 */
import type { AcpCapabilities, AcpConfigChoice, AcpMode } from "@moon/shared";

/** agent 宣告的会话模式列表（权限/思考合一的会话级模式）。 */
export function acpSessionModes(
  capabilities: AcpCapabilities | null,
): AcpMode[] {
  return capabilities?.modes ?? [];
}

/**
 * 识别 agent 宣告的思考类配置项；无思考选项时返回 null。
 *
 * 规则：category 或 id 不区分大小写含 "think" 或 "reason"。
 */
export function acpReasoningOption(
  capabilities: AcpCapabilities | null,
): AcpConfigChoice[] | null {
  const option = capabilities?.configOptions.find((candidate) => {
    const id = candidate.id.toLowerCase();
    const category = (candidate.category ?? "").toLowerCase();
    return (
      id.includes("think") ||
      id.includes("reason") ||
      category.includes("think") ||
      category.includes("reason")
    );
  });
  return option ? option.options : null;
}
