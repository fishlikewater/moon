import { Link } from "react-router-dom";
import type { AcpFormAvailability } from "../pages/agentAcpForm";

/** 每个状态给出下一步动作；未选中的可用条目不必强引导但保留入口。 */
function HarnessLink({ prefix }: { prefix: string }) {
  return (
    <Link className="agent-form-guide-link" to="/harnesses">
      {prefix}前往 Harness 页面
    </Link>
  );
}

/**
 * 智能体表单内的 ACP 接入引导。
 *
 * 只负责展示，状态判定在 `resolveAcpFormAvailability`；注册表读取中、
 * 读取失败、为空、无可用条目与选中项未检测/失败都在这里给出原因和入口。
 */
export function AcpFormGuide({
  availability,
}: {
  availability: AcpFormAvailability;
}) {
  let tone: "info" | "warn" | "error" | "ready" = "info";
  let message: string;
  let showLink = true;

  switch (availability.kind) {
    case "loading":
      message = "正在读取 ACP 注册表…";
      break;
    case "load-error":
      tone = "error";
      message = `ACP 注册表读取失败：${availability.message}`;
      break;
    case "empty":
      tone = "warn";
      message = "注册表为空：还没有注册任何 ACP agent。";
      break;
    case "unselected":
      if (availability.usableCount > 0) {
        message = `注册表中有 ${availability.usableCount} 个可用 ACP agent；选择后可查看其模型与会话模式。`;
      } else if (availability.pendingCount > 0) {
        tone = "warn";
        message = `注册表中暂无可用条目：${availability.pendingCount} 个尚未检测能力，打开 Harness 页面检测后才能确认可用性。`;
      } else {
        tone = "error";
        message = `注册表中的条目均不可用：${availability.firstError ?? "未知原因"}`;
      }
      break;
    case "selected-missing":
      tone = "warn";
      message = "选中的 ACP agent 已不在注册表中，请重新选择。";
      break;
    case "selected-pending":
      tone = "warn";
      message = `「${availability.name}」尚未检测能力；在 Harness 页面点「检测能力」，探测成功后才能选择模型与会话模式。`;
      break;
    case "selected-error":
      tone = "error";
      message = `「${availability.name}」不可用：${availability.error}`;
      break;
    case "selected-ready":
      tone = "ready";
      showLink = false;
      message = `「${availability.name}」已探测：${availability.modelCount} 个模型、${availability.modeCount} 个会话模式。`;
      break;
  }

  return (
    <div
      className={`agent-form-guide agent-form-guide-${tone}`}
      role={tone === "error" ? "alert" : "status"}
    >
      <span>{message}</span>
      {showLink && <HarnessLink prefix="下一步：" />}
    </div>
  );
}
