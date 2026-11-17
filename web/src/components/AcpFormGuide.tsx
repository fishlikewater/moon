import { Link } from "react-router-dom";
import type { AcpFormAvailability } from "../pages/agentAcpForm";

/** 每个状态给出下一步动作；未选中的可用条目不必强引导但保留入口。 */
function HarnessLink({ label }: { label: string }) {
  return (
    <Link className="agent-form-guide-link" to="/harnesses">
      {label}
    </Link>
  );
}

/**
 * 智能体表单内的 ACP 接入引导。
 *
 * 只负责展示，状态判定在 `resolveAcpFormAvailability`；注册表读取中、
 * 读取失败、为空、无可用条目与选中项未检测/失败都在这里给出原因和入口。
 * 选中的未就绪条目额外提供重新检测动作与排查顺序。
 */
export function AcpFormGuide({
  availability,
  onReprobe,
  reprobeing = false,
  reprobeError = null,
}: {
  availability: AcpFormAvailability;
  /** 重新探测当前选中的 ACP agent；未提供时不渲染动作按钮。 */
  onReprobe?: () => void;
  reprobeing?: boolean;
  reprobeError?: string | null;
}) {
  let tone: "info" | "warn" | "error" | "ready" = "info";
  let message: string;
  let showLink = true;
  let showReprobe = false;
  let reprobeLabel = "重新检测";
  let checkOrder: string | null = null;

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
      showReprobe = onReprobe !== undefined;
      reprobeLabel = "立即检测";
      message = `「${availability.name}」尚未检测能力；探测成功后才能选择模型与会话模式。`;
      break;
    case "selected-error":
      tone = "error";
      showReprobe = onReprobe !== undefined;
      message = `「${availability.name}」不可用：${availability.error}`;
      checkOrder =
        "检查顺序：命令与 PATH → Node.js 运行时 → CLI 登录/环境变量 → 模型与供应商。";
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
      {checkOrder !== null && <span className="field-hint">{checkOrder}</span>}
      {reprobeError !== null && (
        <span className="field-hint" role="alert">
          重新检测失败：{reprobeError}
        </span>
      )}
      {showReprobe && (
        <div className="agent-actions">
          <button
            type="button"
            className="btn btn-secondary btn-sm"
            onClick={onReprobe}
            disabled={reprobeing}
          >
            {reprobeing ? "检测中…" : reprobeLabel}
          </button>
          <HarnessLink label="打开 Harness 页面修复" />
        </div>
      )}
      {showLink && !showReprobe && (
        <HarnessLink label="下一步：前往 Harness 页面" />
      )}
    </div>
  );
}
