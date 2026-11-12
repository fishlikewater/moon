import type { HarnessCatalogState } from "../harnessCatalog";
import {
  BUILTIN_HARNESSES,
  HARNESS_STATUS_LABELS,
  catalogStatusText,
} from "../harnessCatalog";

interface HarnessOverviewProps {
  /** 按 harness kind 索引的模型目录状态；缺失按“读取中”展示。 */
  states: Record<string, HarnessCatalogState>;
  /** 当前 ACP 注册表条目数；由页面从注册表组件回调更新。 */
  acpCount: number;
}

/** 内置 harness 概览卡片：只展示静态元数据与目录状态，不发起请求。 */
export function HarnessOverview({ states, acpCount }: HarnessOverviewProps) {
  return (
    <section
      className="card harness-overview"
      aria-labelledby="builtin-harness-title"
    >
      <header className="harness-section-head">
        <h2 className="card-title" id="builtin-harness-title">
          内置 harness
        </h2>
        <p className="field-hint">
          状态来自本机 harness 配置与 ACP 注册表；模型目录为只读读取。
        </p>
      </header>

      <ul className="harness-grid" aria-label="内置 harness 列表">
        {BUILTIN_HARNESSES.map((info) => {
          const modelHint =
            info.kind === "acp"
              ? `注册表 ${acpCount} 个 agent；${catalogStatusText(info, undefined)}`
              : catalogStatusText(info, states[info.kind]);
          return (
            <li key={info.kind} className="harness-card">
              <div className="harness-card-head">
                <h3 className="harness-card-name">{info.label}</h3>
                <span
                  className={`badge harness-badge is-${info.status}`}
                  title={`运行状态：${HARNESS_STATUS_LABELS[info.status]}`}
                >
                  {HARNESS_STATUS_LABELS[info.status]}
                </span>
              </div>
              <p className="harness-card-transport">{info.transport}</p>
              <div className="harness-capability-list">
                {info.capabilities.map((capability) => (
                  <span key={capability} className="harness-capability">
                    {capability}
                  </span>
                ))}
              </div>
              <p className="field-hint harness-card-model">{modelHint}</p>
            </li>
          );
        })}
      </ul>
    </section>
  );
}
