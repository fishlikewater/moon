import { useEffect, useRef, useState, type FormEvent } from "react";
import { createPortal } from "react-dom";
import type { ApiProvider } from "@moon/shared";
import { api, errorMessage } from "../api/client";
import {
  emptyModelRow,
  PROVIDER_MODEL_INPUT_TYPES,
  PROVIDER_PROTOCOL_OPTIONS,
  providerFormFromItem,
  serializeProviderForm,
  validateProviderForm,
  type ModelRowState,
  type ProviderFormState,
} from "./providerForm";

interface ApiProviderFormModalProps {
  /** null 表示新建；否则编辑该条目。 */
  item: ApiProvider | null;
  onClose: () => void;
  onSaved: () => void;
}

const FOCUSABLE_SELECTOR = [
  "button:not([disabled])",
  "input:not([disabled])",
  "textarea:not([disabled])",
  "select:not([disabled])",
  "a[href]",
  '[tabindex]:not([tabindex="-1"])',
].join(",");

/**
 * 模型供应商新建/编辑表单弹层。
 *
 * 表单状态与保存请求都留在弹层内；注册表只负责打开/关闭与保存后刷新。
 * 弹层行为与 AcpHarnessFormModal 同款：挂 body、锁定滚动、Tab 焦点陷阱、
 * 关闭归还触发按钮焦点。
 */
export function ApiProviderFormModal({
  item,
  onClose,
  onSaved,
}: ApiProviderFormModalProps) {
  const [form, setForm] = useState<ProviderFormState>(() =>
    providerFormFromItem(item),
  );
  const [formError, setFormError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const dialogRef = useRef<HTMLDivElement | null>(null);
  const nameInputRef = useRef<HTMLInputElement | null>(null);
  const savingRef = useRef(saving);
  const closeRef = useRef(onClose);

  useEffect(() => {
    closeRef.current = onClose;
  }, [onClose]);

  useEffect(() => {
    savingRef.current = saving;
  }, [saving]);

  useEffect(() => {
    const trigger =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    nameInputRef.current?.focus();

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        if (savingRef.current) {
          return;
        }
        event.preventDefault();
        closeRef.current();
        return;
      }
      if (event.key !== "Tab") {
        return;
      }

      const container = dialogRef.current;
      if (container === null) {
        return;
      }
      const focusables = Array.from(
        container.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR),
      );
      if (focusables.length === 0) {
        event.preventDefault();
        container.focus();
        return;
      }

      const first = focusables[0];
      const last = focusables[focusables.length - 1];
      const active = document.activeElement;
      if (active === container) {
        event.preventDefault();
        (event.shiftKey ? last : first).focus();
        return;
      }
      const inside = active instanceof Node && container.contains(active);
      if (event.shiftKey) {
        if (!inside || active === first) {
          event.preventDefault();
          last.focus();
        }
      } else if (!inside || active === last) {
        event.preventDefault();
        first.focus();
      }
    };

    document.addEventListener("keydown", handleKeyDown, true);
    return () => {
      document.removeEventListener("keydown", handleKeyDown, true);
      document.body.style.overflow = previousOverflow;
      if (trigger?.isConnected) {
        trigger.focus();
      }
    };
  }, []);

  const patchModelRow = (index: number, patch: Partial<ModelRowState>) => {
    setForm((current) => ({
      ...current,
      models: current.models.map((row, row_index) =>
        row_index === index ? { ...row, ...patch } : row,
      ),
    }));
  };

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const errors = validateProviderForm(form);
    if (errors.length > 0) {
      setFormError(errors.join("；"));
      return;
    }

    savingRef.current = true;
    setSaving(true);
    setFormError(null);
    try {
      const payload = serializeProviderForm(form);
      if (form.editingId === null) {
        await api.apiProviders.create(payload);
      } else {
        await api.apiProviders.update(form.editingId, payload);
      }
      savingRef.current = false;
      setSaving(false);
      onSaved();
    } catch (cause) {
      savingRef.current = false;
      setSaving(false);
      setFormError(errorMessage(cause));
    }
  };

  const title =
    item === null ? "新建模型供应商" : `编辑模型供应商：${item.name}`;

  return createPortal(
    <div className="modal-layer">
      <div
        className="modal-card"
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby="api-provider-form-title"
        tabIndex={-1}
      >
        <header className="modal-head">
          <h2 id="api-provider-form-title" className="card-title">
            {title}
          </h2>
          <button
            type="button"
            className="btn btn-ghost btn-sm"
            aria-label="关闭模型供应商表单"
            onClick={onClose}
            disabled={saving}
          >
            关闭
          </button>
        </header>

        <form className="form-stack" onSubmit={submit}>
          <p className="field-hint">
            模型供应商保存 Base URL 与 API Key；智能体（ACP harness）绑定后，
            Moon 会在启动时按协议自动注入 ANTHROPIC_BASE_URL /
            ANTHROPIC_AUTH_TOKEN 或 OPENAI_BASE_URL / OPENAI_API_KEY。
          </p>

          <div className="form-grid">
            <label className="field">
              <span className="field-label">名称</span>
              <input
                ref={nameInputRef}
                required
                value={form.name}
                onChange={(event) =>
                  setForm((current) => ({
                    ...current,
                    name: event.target.value,
                  }))
                }
                placeholder="例如：DeepSeek"
              />
            </label>

            <label className="field">
              <span className="field-label">协议</span>
              <select
                value={form.protocol}
                onChange={(event) =>
                  setForm((current) => ({
                    ...current,
                    protocol: event.target.value as ProviderFormState["protocol"],
                  }))
                }
              >
                {PROVIDER_PROTOCOL_OPTIONS.map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </select>
            </label>

            <label className="field form-span-2">
              <span className="field-label">Base URL</span>
              <input
                required
                value={form.baseUrl}
                onChange={(event) =>
                  setForm((current) => ({
                    ...current,
                    baseUrl: event.target.value,
                  }))
                }
                placeholder="https://api.deepseek.com/anthropic"
              />
            </label>

            <label className="field form-span-2">
              <span className="field-label">API Key（本地明文存储）</span>
              <input
                type="password"
                autoComplete="off"
                value={form.apiKey}
                onChange={(event) =>
                  setForm((current) => ({
                    ...current,
                    apiKey: event.target.value,
                  }))
                }
                placeholder="sk-…"
              />
            </label>

            <label className="field form-span-2 field-checkbox">
              <input
                type="checkbox"
                checked={form.enabled}
                onChange={(event) =>
                  setForm((current) => ({
                    ...current,
                    enabled: event.target.checked,
                  }))
                }
              />
              <span>启用该供应商（停用后绑定的智能体运行会报错）</span>
            </label>
          </div>

          <fieldset className="provider-models">
            <legend className="field-label">模型与高级属性</legend>
            <p className="field-hint">
              上下文大小单位为 tokens；思考等级用逗号分隔（如 standard,
              deep）；输入类型决定该模型可接收的内容形态。
            </p>
            {form.models.map((row, index) => (
              <div className="provider-model-row" key={index}>
                <div className="provider-model-fields">
                  <label className="field">
                    <span className="field-label">模型 id</span>
                    <input
                      value={row.id}
                      onChange={(event) =>
                        patchModelRow(index, { id: event.target.value })
                      }
                      placeholder="deepseek-chat"
                      aria-label={`模型 ${index + 1} id`}
                    />
                  </label>
                  <label className="field">
                    <span className="field-label">显示名</span>
                    <input
                      value={row.name}
                      onChange={(event) =>
                        patchModelRow(index, { name: event.target.value })
                      }
                      placeholder="留空则与 id 相同"
                      aria-label={`模型 ${index + 1} 显示名`}
                    />
                  </label>
                  <label className="field">
                    <span className="field-label">上下文大小</span>
                    <input
                      inputMode="numeric"
                      value={row.contextWindowText}
                      onChange={(event) =>
                        patchModelRow(index, {
                          contextWindowText: event.target.value,
                        })
                      }
                      placeholder="65536"
                      aria-label={`模型 ${index + 1} 上下文大小`}
                    />
                  </label>
                  <label className="field">
                    <span className="field-label">思考等级（逗号分隔）</span>
                    <input
                      value={row.reasoningText}
                      onChange={(event) =>
                        patchModelRow(index, {
                          reasoningText: event.target.value,
                        })
                      }
                      placeholder="standard, deep"
                      aria-label={`模型 ${index + 1} 思考等级`}
                    />
                  </label>
                </div>
                <div
                  className="provider-model-input-types"
                  role="group"
                  aria-label={`模型 ${index + 1} 输入类型`}
                >
                  {PROVIDER_MODEL_INPUT_TYPES.map((type) => (
                    <label key={type} className="field-checkbox">
                      <input
                        type="checkbox"
                        checked={row.inputTypes.includes(type)}
                        onChange={(event) => {
                          const next = event.target.checked
                            ? [...row.inputTypes, type]
                            : row.inputTypes.filter((value) => value !== type);
                          patchModelRow(index, { inputTypes: next });
                        }}
                      />
                      <span>
                        {type === "text"
                          ? "文本"
                          : type === "image"
                            ? "图片"
                            : "视频"}
                      </span>
                    </label>
                  ))}
                  <button
                    type="button"
                    className="btn btn-ghost btn-sm"
                    onClick={() =>
                      setForm((current) => ({
                        ...current,
                        models: current.models.filter(
                          (_, row_index) => row_index !== index,
                        ),
                      }))
                    }
                    disabled={saving}
                    aria-label={`删除模型 ${index + 1}`}
                  >
                    删除此模型
                  </button>
                </div>
              </div>
            ))}
            <button
              type="button"
              className="btn btn-secondary btn-sm"
              onClick={() =>
                setForm((current) => ({
                  ...current,
                  models: [...current.models, emptyModelRow()],
                }))
              }
              disabled={saving}
            >
              添加模型
            </button>
          </fieldset>

          {formError !== null && (
            <p className="banner error" role="alert">
              {formError}
            </p>
          )}

          <div className="form-actions">
            <button type="submit" className="btn btn-primary" disabled={saving}>
              {saving
                ? "保存中…"
                : form.editingId === null
                  ? "创建"
                  : "保存修改"}
            </button>
            <button
              type="button"
              className="btn btn-secondary"
              onClick={onClose}
              disabled={saving}
            >
              取消
            </button>
          </div>
        </form>
      </div>
    </div>,
    document.body,
  );
}
