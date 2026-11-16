import { useCallback, useEffect, useRef, useState } from "react";
import type { ApiProvider, ProviderProtocol } from "@moon/shared";
import { api, errorMessage } from "../api/client";
import { ApiProviderFormModal } from "./ApiProviderFormModal";

interface ApiProviderRegistryProps {
  /** 列表变化时回传最新条目（供 agent 表单联动消费）。 */
  onChanged?: (items: ApiProvider[]) => void;
}

/** 协议徽标文案；未识别协议按原样显示。 */
export function protocolLabel(protocol: ProviderProtocol): string {
  return protocol === "anthropic"
    ? "Anthropic 兼容"
    : protocol === "openai"
      ? "OpenAI 兼容"
      : protocol;
}

/**
 * 模型供应商注册表：列表、新建/编辑弹层与删除。
 *
 * 与 ACP 注册表同款交互：列表内联、表单弹层、保存后静默刷新、
 * 被引用条目删除禁用 + 服务端 409 兜底。
 */
export function ApiProviderRegistry({ onChanged }: ApiProviderRegistryProps) {
  const [items, setItems] = useState<ApiProvider[]>([]);
  const [loading, setLoading] = useState(true);
  const [listError, setListError] = useState<string | null>(null);
  const [deletingId, setDeletingId] = useState<number | null>(null);
  const [formTarget, setFormTarget] = useState<ApiProvider | "create" | null>(
    null,
  );
  const containerRef = useRef<HTMLElement | null>(null);

  const reload = useCallback(
    async (options?: { showLoading?: boolean }) => {
      if (options?.showLoading !== false) {
        setLoading(true);
      }
      setListError(null);
      try {
        const next = await api.apiProviders.list();
        setItems(next);
        onChanged?.(next);
      } catch (cause) {
        setListError(errorMessage(cause));
      } finally {
        setLoading(false);
      }
    },
    [onChanged],
  );

  useEffect(() => {
    void reload();
  }, [reload]);

  const closeForm = useCallback(() => {
    setFormTarget(null);
  }, []);

  const handleFormSaved = useCallback(() => {
    setFormTarget(null);
    // 保存后静默刷新：列表保持挂载，弹层卸载时触发按钮仍在，焦点才能归还。
    void reload({ showLoading: false });
  }, [reload]);

  const remove = async (item: ApiProvider) => {
    if (!window.confirm(`确认删除模型供应商「${item.name}」？`)) {
      return;
    }
    setListError(null);
    setDeletingId(item.id);
    try {
      await api.apiProviders.remove(item.id);
      await reload();
      containerRef.current?.focus();
    } catch (cause) {
      setListError(errorMessage(cause));
    } finally {
      setDeletingId(null);
    }
  };

  return (
    <section
      className="card harness-registry"
      id="api-providers"
      aria-labelledby="api-providers-title"
      ref={containerRef}
      tabIndex={-1}
    >
      <header className="harness-section-head">
        <h2 className="card-title" id="api-providers-title">
          模型供应商
        </h2>
        <button
          type="button"
          className="btn btn-primary btn-sm"
          onClick={() => setFormTarget("create")}
        >
          新建供应商
        </button>
      </header>

      <p className="field-hint">
        供应商保存 Base URL 与 API Key；ACP 智能体绑定后，启动时按协议自动注入
        环境变量，绑定即生效。
      </p>

      {listError !== null && (
        <p className="banner error" role="alert">
          {listError}
        </p>
      )}

      {loading ? (
        <p className="field-hint" role="status">
          正在读取供应商…
        </p>
      ) : items.length === 0 ? (
        <p className="field-hint">
          尚无供应商：点击「新建供应商」添加 OpenAI / Anthropic 兼容服务。
        </p>
      ) : (
        <ul className="acp-harness-list" aria-label="模型供应商列表">
          {items.map((item) => {
            const inUse = item.agentCount > 0;
            return (
              <li key={item.id} className="acp-harness-item">
                <div className="acp-harness-body">
                  <div className="acp-harness-title-row">
                    <h3 className="acp-harness-name">{item.name}</h3>
                    <span className="badge">
                      {protocolLabel(item.protocol)}
                    </span>
                    {item.enabled ? (
                      <span className="badge badge-ready">已启用</span>
                    ) : (
                      <span className="badge badge-error">已停用</span>
                    )}
                    <span className="badge">
                      {inUse ? `绑定 ${item.agentCount}` : "未绑定"}
                    </span>
                    <span className="badge">模型 {item.models.length}</span>
                  </div>
                  <code className="acp-harness-command">{item.baseUrl}</code>
                  {item.models.length > 0 && (
                    <p className="field-hint">
                      {item.models
                        .map((model) => {
                          const window =
                            model.contextWindow === null
                              ? ""
                              : ` · ${model.contextWindow} tokens`;
                          return `${model.name}${window}`;
                        })
                        .join("；")}
                    </p>
                  )}
                  {inUse && (
                    <p className="field-hint">
                      正被 {item.agentCount} 个智能体绑定，先改绑这些智能体后才能删除。
                    </p>
                  )}
                  <div className="acp-harness-actions">
                    <button
                      type="button"
                      className="btn btn-secondary btn-sm"
                      onClick={() => setFormTarget(item)}
                      disabled={deletingId !== null}
                    >
                      编辑
                    </button>
                    <button
                      type="button"
                      className="btn btn-danger btn-sm"
                      onClick={() => void remove(item)}
                      disabled={deletingId !== null || inUse}
                      aria-label={`删除供应商 ${item.name}`}
                    >
                      {deletingId === item.id ? "删除中…" : "删除"}
                    </button>
                  </div>
                </div>
              </li>
            );
          })}
        </ul>
      )}

      {formTarget !== null && (
        <ApiProviderFormModal
          item={formTarget === "create" ? null : formTarget}
          onClose={closeForm}
          onSaved={handleFormSaved}
        />
      )}
    </section>
  );
}
