import type { AcpCapabilities } from "@moon/shared";
import { CapabilityChips } from "./acpCapabilityChips";

/** epoch 毫秒 → 本地时间文本；未探测为「未知」。 */
export function formatProbedAt(value: number | null): string {
  if (value === null) {
    return "未知";
  }
  return new Date(value).toLocaleString("zh-CN", { hour12: false });
}

type CapabilityDetailProps = {
  name: string;
  capability: AcpCapabilities | null;
  probedAt: number | null;
  probeError: string | null;
  onBack: () => void;
};

/** 探测能力详情视图：模型/权限 chips、探测时间、未探测占位与错误提示。 */
export function AcpCapabilityDetail({
  name,
  capability,
  probedAt,
  probeError,
  onBack,
}: CapabilityDetailProps) {
  return (
    <div className="acp-capability-detail">
      <header className="acp-capability-head">
        <button
          type="button"
          className="btn btn-secondary btn-sm"
          onClick={onBack}
        >
          返回列表
        </button>
        <h3 className="acp-capability-name">{name}</h3>
      </header>
      {probeError !== null && (
        <p className="field-error" role="alert">
          {probeError}
        </p>
      )}
      {capability === null ? (
        <p className="field-hint">
          尚未探测模型与权限模式；运行仍使用 agent 默认值。
        </p>
      ) : (
        <div className="acp-harness-capabilities">
          <CapabilityChips
            label="模型"
            values={capability.models.map((model) => model.value)}
            currentValue={capability.currentModelId}
          />
          <CapabilityChips
            label="权限模式"
            values={capability.modes.map((mode) => mode.name)}
            currentValue={
              capability.modes.find(
                (mode) => mode.id === capability.currentModeId,
              )?.name ?? null
            }
          />
          {capability.currentModelId === null && (
            <p className="field-hint">默认模型未声明。</p>
          )}
          <p className="field-hint">探测时间：{formatProbedAt(probedAt)}</p>
        </div>
      )}
    </div>
  );
}
