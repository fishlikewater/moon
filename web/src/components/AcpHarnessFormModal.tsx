import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type FormEvent,
} from "react";
import { createPortal } from "react-dom";
import type { CustomHarness } from "@moon/shared";
import { api, errorMessage } from "../api/client";
import {
  harnessFormFromItem,
  parseArgsText,
  parseEnvText,
  type HarnessFormState,
} from "./acpHarnessForm";
import {
  ACP_HARNESS_TEMPLATES,
  templateFormValues,
  type AcpHarnessTemplate,
} from "./acpHarnessTemplates";
import {
  registryPresetValues,
  type AcpRegistryAgent,
} from "../acpRegistryCatalog";
import type { AcpRegistryState } from "../useAcpRegistry";

interface AcpHarnessFormModalProps {
  /** null 表示新建；否则编辑该条目。 */
  item: CustomHarness | null;
  /** ACP registry 目录状态：驱动预设选择器与失败回退。 */
  registry: AcpRegistryState;
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
 * ACP agent 新建/编辑表单弹层。
 *
 * 表单状态、模板与保存请求都留在弹层内；注册表只负责打开/关闭与保存后刷新。
 * 弹层挂到 body，避免页面滚动容器影响覆盖层；打开期间锁定 body 滚动，
 * 并把 Tab 焦点限制在弹层内部。
 */
export function AcpHarnessFormModal({
  item,
  registry,
  onClose,
  onSaved,
}: AcpHarnessFormModalProps) {
  const [form, setForm] = useState<HarnessFormState>(() =>
    harnessFormFromItem(item),
  );
  const [formError, setFormError] = useState<string | null>(null);
  /** 占位模板/需本机安装预设点击后的提示；预填命令的预设清空它。 */
  const [templateNotice, setTemplateNotice] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [presetQuery, setPresetQuery] = useState("");

  // 预设排序：可用分发（npx 或二进制）在前且 npx 优先；两类都缺失的条目
  // 没有可预填的命令，不作为预设展示。
  const sortedAgents = useMemo(() => {
    if (registry.status !== "ready") {
      return [];
    }
    const byName = (a: AcpRegistryAgent, b: AcpRegistryAgent) =>
      a.name.localeCompare(b.name, "zh-Hans-CN");
    const npx = registry.payload.agents
      .filter((agent) => agent.npx !== null)
      .sort(byName);
    const binaries = registry.payload.agents
      .filter((agent) => agent.npx === null && agent.windowsBinary !== null)
      .sort(byName);
    return [...npx, ...binaries];
  }, [registry]);

  const filteredAgents = useMemo(() => {
    const query = presetQuery.trim().toLowerCase();
    if (query === "") {
      return sortedAgents;
    }
    return sortedAgents.filter(
      (agent) =>
        agent.name.toLowerCase().includes(query) ||
        agent.id.toLowerCase().includes(query) ||
        agent.summary.toLowerCase().includes(query),
    );
  }, [sortedAgents, presetQuery]);

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
      // 卡片自身 tabIndex=-1 持有焦点时，Shift+Tab 也必须留在弹层内。
      if (active === container) {
        event.preventDefault();
        (event.shiftKey ? last : first).focus();
        return;
      }
      const inside =
        active instanceof Node && container.contains(active);
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

  // 模板只覆盖名称/命令/参数；编辑时保留用户已填的环境变量。
  // 占位模板（ACP 入口未核实）只预填名称，命令需自行填写。
  const applyTemplate = (template: AcpHarnessTemplate) => {
    const platform =
      typeof navigator === "undefined" ? "" : navigator.userAgent;
    setForm((current) => ({
      ...current,
      ...templateFormValues(template, platform),
    }));
    setFormError(null);
    setTemplateNotice(
      template.prefillCommand
        ? null
        : `「${template.label}」的 ACP 入口未核实，已预填名称；请自行填写启动命令与参数（参数每行一个）。`,
    );
  };

  // registry 预设：npx 条目预填锁版本命令；二进制条目只预填裸命令并提示需本机安装。
  const applyRegistryPreset = (agent: AcpRegistryAgent) => {
    const platform =
      typeof navigator === "undefined" ? "" : navigator.userAgent;
    const values = registryPresetValues(agent, platform);
    setForm((current) => ({
      ...current,
      ...values,
    }));
    setFormError(null);
    setTemplateNotice(
      agent.npx !== null
        ? null
        : `「${agent.name}」需要本机安装后才能运行；已预填名称与命令「${values.command}」，请确认 PATH 可用。`,
    );
  };

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const name = form.name.trim();
    const command = form.command.trim();
    if (name === "") {
      setFormError("名称不能为空");
      return;
    }
    if (command === "") {
      setFormError("启动命令不能为空");
      return;
    }

    const args = parseArgsText(form.argsText);
    const { env, errors } = parseEnvText(form.envText);
    if (errors.length > 0) {
      setFormError(errors.join("；"));
      return;
    }

    savingRef.current = true;
    setSaving(true);
    setFormError(null);
    try {
      const payload = { name, command, args, env };
      if (form.editingId === null) {
        await api.customHarnesses.create(payload);
      } else {
        await api.customHarnesses.update(form.editingId, payload);
      }
      savingRef.current = false;
      setSaving(false);
      // 成功即关闭弹层；列表刷新由注册表的 onSaved 负责。
      onSaved();
    } catch (cause) {
      savingRef.current = false;
      setSaving(false);
      setFormError(errorMessage(cause));
    }
  };

  const title =
    item === null ? "新建 ACP agent" : `编辑 ACP agent：${item.name}`;

  return createPortal(
    <div className="modal-layer">
      <div
        className="modal-card"
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby="acp-harness-form-title"
        tabIndex={-1}
      >
        <header className="modal-head">
          <h2 id="acp-harness-form-title" className="card-title">
            {title}
          </h2>
          <button
            type="button"
            className="btn btn-ghost btn-sm"
            aria-label="关闭 ACP agent 表单"
            onClick={onClose}
            disabled={saving}
          >
            关闭
          </button>
        </header>

        <form className="form-stack" onSubmit={submit}>
          <p className="field-hint">
            自定义 ACP agent 是本地可执行的 ACP 程序；参数每行一个，环境变量每行
            KEY=VALUE。
          </p>

          <div className="harness-templates">
            <span className="field-label">ACP 注册表预设</span>
            {registry.status === "loading" && (
              <p className="field-hint" role="status">
                正在读取 ACP 注册表…
              </p>
            )}
            {registry.status === "error" && (
              <>
                <p className="field-hint" role="status">
                  注册表读取失败（{registry.message}
                  ），已回退内置模板。
                </p>
                <div className="harness-template-list">
                  {ACP_HARNESS_TEMPLATES.map((template) => (
                    <button
                      key={template.id}
                      type="button"
                      className={`harness-template${
                        template.prefillCommand ? "" : " is-placeholder"
                      }`}
                      onClick={() => applyTemplate(template)}
                      disabled={saving}
                      title={template.description}
                    >
                      <span className="harness-template-label">
                        {template.label}
                      </span>
                    </button>
                  ))}
                </div>
              </>
            )}
            {registry.status === "ready" && (
              <>
                <input
                  type="search"
                  className="input acp-registry-search"
                  aria-label="搜索 ACP 注册表预设"
                  placeholder="搜索预设（名称 / id / 说明）"
                  value={presetQuery}
                  onChange={(event) => setPresetQuery(event.target.value)}
                />
                {filteredAgents.length === 0 ? (
                  <p className="field-hint">没有匹配的预设。</p>
                ) : (
                  <ul
                    className="acp-registry-preset-list"
                    aria-label="ACP 注册表预设列表"
                  >
                    {filteredAgents.map((agent) => (
                      <li key={agent.id}>
                        <button
                          type="button"
                          className="acp-registry-preset"
                          onClick={() => applyRegistryPreset(agent)}
                          disabled={saving}
                          title={agent.summary}
                        >
                          <span className="acp-registry-preset-head">
                            <span className="acp-registry-preset-name">
                              {agent.name}
                            </span>
                            <span className="acp-registry-preset-version">
                              v{agent.version}
                            </span>
                            {agent.npx === null && (
                              <span className="badge">需本机安装</span>
                            )}
                          </span>
                          {agent.summary !== "" && (
                            <span className="acp-registry-preset-summary">
                              {agent.summary}
                            </span>
                          )}
                        </button>
                      </li>
                    ))}
                  </ul>
                )}
                <p className="field-hint">
                  点击 npx 条目预填 npx 命令与锁版本参数；「需本机安装」条目只预填名称与命令，
                  需先自行安装对应 CLI。首次使用仍需在对应 CLI 完成登录或配置 API
                  Key。Windows 下 npx 命令为 npx.cmd。
                </p>
              </>
            )}
            {templateNotice !== null && (
              <p className="field-hint" role="status">
                {templateNotice}
              </p>
            )}
          </div>

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
                placeholder="例如：gemini-cli"
              />
            </label>

            <label className="field">
              <span className="field-label">启动命令</span>
              <input
                required
                value={form.command}
                onChange={(event) =>
                  setForm((current) => ({
                    ...current,
                    command: event.target.value,
                  }))
                }
                placeholder="node / npx.cmd / opencode"
              />
            </label>

            <label className="field form-span-2">
              <span className="field-label">参数（每行一个）</span>
              <textarea
                rows={3}
                aria-label="参数，每行一个"
                value={form.argsText}
                onChange={(event) =>
                  setForm((current) => ({
                    ...current,
                    argsText: event.target.value,
                  }))
                }
                placeholder={"agent.mjs\n--stdio"}
              />
            </label>

            <label className="field form-span-2">
              <span className="field-label">环境变量（每行 KEY=VALUE）</span>
              <textarea
                rows={3}
                aria-label="环境变量，每行 KEY=VALUE"
                value={form.envText}
                onChange={(event) =>
                  setForm((current) => ({
                    ...current,
                    envText: event.target.value,
                  }))
                }
                placeholder={"ACP_DEBUG=1\nAPI_KEY=abc=def"}
              />
            </label>
          </div>

          <p className="field-hint">
            Windows 下通过 npx 启动的 agent 请写 npx.cmd；环境变量的值可以包含
            等号。
          </p>

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
