import { ApiProviderRegistry } from "../components/ApiProviderRegistry";
import { PageHeader } from "../components/layout/PageHeader";

/** 模型页面：管理 OpenAI / Anthropic 兼容的模型供应商。 */
export default function ModelsPage() {
  return (
    <div className="page models-page">
      <PageHeader
        title="模型"
        subtitle="配置模型供应商的 Base URL、API Key 与协议；ACP 智能体绑定后启动时自动注入。"
      />
      <ApiProviderRegistry />
    </div>
  );
}
