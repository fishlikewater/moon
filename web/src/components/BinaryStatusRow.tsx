import type { HarnessBinaryStatus } from "@moon/shared";
import { binarySourceLabel, binaryStatus } from "../settingsView";

/** 单个 harness 的解析结论：状态徽标 + 来源 + 实际路径。 */
export function BinaryStatusRow({
  name,
  status,
}: {
  name: string;
  status: HarnessBinaryStatus | undefined;
}) {
  if (status === undefined) {
    return (
      <div className="settings-binary-row">
        <div className="settings-binary-head">
          <span className="settings-binary-name">{name}</span>
        </div>
        <span className="settings-binary-path">尚未检测</span>
      </div>
    );
  }
  const state = binaryStatus(status);
  return (
    <div className="settings-binary-row">
      <div className="settings-binary-head">
        <span className="settings-binary-name">{name}</span>
        <span
          className={`badge ${state.tone === "ok" ? "badge-ready" : "badge-warn"}`}
        >
          {state.label}
        </span>
        <span className="badge badge-harness">{binarySourceLabel(status.source)}</span>
      </div>
      <span className="settings-binary-path">{status.path.trim() || "—"}</span>
      {status.searched.length > 0 && (
        <span className="field-hint">已尝试：{status.searched.join("、")}</span>
      )}
    </div>
  );
}
