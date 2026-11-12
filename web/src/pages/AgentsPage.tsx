import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type FormEvent,
} from "react";
import { Link, useSearchParams } from "react-router-dom";
import type {
  Agent,
  CustomHarness,
  HarnessKind,
  HarnessModelCatalog,
  HarnessModelOption,
  ModelConfig,
} from "@moon/shared";
import { api, errorMessage } from "../api/client";
import { acpReasoningOption, acpSessionModes } from "../acpAgentCapabilities";
import { AgentAvatar } from "../components/AgentAvatar";
import { EmptyState } from "../components/layout/EmptyState";
import { ListSkeleton } from "../components/layout/ListSkeleton";
import { PageHeader } from "../components/layout/PageHeader";
import { shouldOpenNew, withoutNewParam } from "./newEntry";

const HARNESS_OPTIONS: ReadonlyArray<{ value: HarnessKind; label: string }> = [
  { value: "zcode", label: "zcode" },
  { value: "codex", label: "codex" },
  { value: "acp", label: "ACP" },
  { value: "kimi-code", label: "kimi-code（暂未支持）" },
  { value: "fake", label: "fake（仅测试）" },
];

/** 旧数据里存在但当前 harness 目录没有的模型，用该值保持可见。 */
const CUSTOM_MODEL_VALUE = "__custom__";
const CUSTOM_REASONING_VALUE = "__custom_reasoning__";
/** agent 不再宣告的会话模式（旧数据）用该值保持可见。 */
const CUSTOM_SESSION_VALUE = "__custom_session__";

interface AgentFormState {
  editingId: number | null;
  name: string;
  persona: string;
  /** 独立工作区；空字符串表示使用房间主工作区。 */
  workspaceDir: string;
  harness: HarnessKind;
  model: string;
  providerId: string;
  /** 模型思考等级；空字符串表示使用模型默认。ACP 时为思考类 configOption 的值。 */
  reasoningLevel: string;
  /** ACP 会话模式（agent 宣告的 mode id）；空字符串表示跟随房间权限模式。 */
  sessionMode: string;
  /** harness 为 acp 时选中的注册表条目；空字符串表示未选择。 */
  customHarnessId: string;
  /** 编辑时保留 modelConfig 的其他字段，避免只改模型却丢掉未知配置。 */
  baseConfig: ModelConfig | null;
}

const EMPTY_FORM: AgentFormState = {
  editingId: null,
  name: "",
  persona: "",
  workspaceDir: "",
  harness: "zcode",
  model: "",
  providerId: "",
  reasoningLevel: "",
  sessionMode: "",
  customHarnessId: "",
  baseConfig: null,
};

interface ModelGroup {
  providerId: string;
  providerName: string;
  models: HarnessModelOption[];
}

/** 保持服务端顺序，按 provider 分组供 optgroup 使用。 */
function groupModels(models: HarnessModelOption[]): ModelGroup[] {
  const groups: ModelGroup[] = [];
  const byProvider = new Map<string, ModelGroup>();

  for (const model of models) {
    let group = byProvider.get(model.providerId);
    if (group === undefined) {
      group = {
        providerId: model.providerId,
        providerName: model.providerName,
        models: [],
      };
      byProvider.set(model.providerId, group);
      groups.push(group);
    }
    group.models.push(model);
  }

  return groups;
}

/** 内联星芒图标：用于内置创建助手入口，不引入图标库。 */
function SparkIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      <path d="M12 3v4" />
      <path d="M12 17v4" />
      <path d="M3 12h4" />
      <path d="M17 12h4" />
      <path d="m5.6 5.6 2.8 2.8" />
      <path d="m15.6 15.6 2.8 2.8" />
      <path d="m18.4 5.6-2.8 2.8" />
      <path d="m8.4 15.6-2.8 2.8" />
    </svg>
  );
}

/** 智能体管理页：列表 + 创建/编辑/删除闭环，加载、空态、错误态齐备。 */
export default function AgentsPage() {
  const [agents, setAgents] = useState<Agent[]>([]);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [form, setForm] = useState<AgentFormState>(EMPTY_FORM);
  const [formOpen, setFormOpen] = useState(false);
  const [catalog, setCatalog] = useState<HarnessModelCatalog | null>(null);
  const [catalogLoading, setCatalogLoading] = useState(true);
  const [catalogError, setCatalogError] = useState<string | null>(null);
  const [customHarnesses, setCustomHarnesses] = useState<CustomHarness[]>([]);
  const modalRef = useRef<HTMLDivElement | null>(null);
  const nameInputRef = useRef<HTMLInputElement | null>(null);

  const startCreate = useCallback(() => {
    setError(null);
    setForm(EMPTY_FORM);
    setFormOpen(true);
  }, []);

  const [searchParams, setSearchParams] = useSearchParams();

  // 命令面板「新建智能体」入口：只消费一次查询参数。
  useEffect(() => {
    if (loading || !shouldOpenNew(searchParams)) {
      return;
    }
    startCreate();
    setSearchParams(withoutNewParam(searchParams), { replace: true });
  }, [loading, searchParams, setSearchParams, startCreate]);

  // 自定义 ACP agent 注册表用于 acp harness 的下拉；失败降级为空列表，
  // 提交时由服务端校验兜底。
  useEffect(() => {
    let cancelled = false;
    api.customHarnesses
      .list()
      .then((items) => {
        if (!cancelled) {
          setCustomHarnesses(items);
        }
      })
      .catch(() => {
        if (!cancelled) {
          setCustomHarnesses([]);
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // 打开模态框时锁定页面滚动并把焦点移到名称输入，关闭后归还焦点。
  useEffect(() => {
    if (!formOpen) {
      return;
    }
    const previouslyFocused =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    // 内置主持人的名称与人设是禁用态，退回到对话框内第一个可聚焦控件。
    if (nameInputRef.current !== null && !nameInputRef.current.disabled) {
      nameInputRef.current.focus();
    } else {
      const fallback = modalRef.current?.querySelector<HTMLElement>(
        "select:not([disabled]), input:not([disabled]), textarea:not([disabled])",
      );
      (fallback ?? modalRef.current)?.focus();
    }

    return () => {
      document.body.style.overflow = previousOverflow;
      previouslyFocused?.focus();
    };
  }, [formOpen]);

  // Esc 关闭（保存中忽略），Tab 在对话框内循环。
  useEffect(() => {
    if (!formOpen) {
      return;
    }
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        if (!saving) {
          setFormOpen(false);
          setForm(EMPTY_FORM);
          setError(null);
        }
        return;
      }
      if (event.key !== "Tab") {
        return;
      }
      const focusable = modalRef.current?.querySelectorAll<HTMLElement>(
        'button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), a[href]',
      );
      if (focusable === undefined || focusable.length === 0) {
        return;
      }
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    };
    document.addEventListener("keydown", handleKeyDown);

    return () => {
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [formOpen, saving]);

  const reload = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setAgents(await api.agents.list());
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  useEffect(() => {
    let cancelled = false;
    setCatalogLoading(true);
    setCatalogError(null);
    setCatalog(null);

    api.harnesses
      .models(form.harness)
      .then((next) => {
        if (!cancelled) {
          setCatalog(next);
        }
      })
      .catch((cause) => {
        if (!cancelled) {
          setCatalogError(errorMessage(cause));
        }
      })
      .finally(() => {
        if (!cancelled) {
          setCatalogLoading(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [form.harness]);

  const selectedCustomHarness =
    form.customHarnessId === ""
      ? undefined
      : customHarnesses.find(
          (item) => String(item.id) === form.customHarnessId,
        );

  // ACP 没有全局模型目录：优先使用注册表里最近一次探测的模型候选。
  const acpCatalogModels = useMemo<HarnessModelOption[]>(() => {
    if (form.harness !== "acp" || selectedCustomHarness?.capabilities == null) {
      return [];
    }
    return selectedCustomHarness.capabilities.models.map((model) => ({
      key: `acp:${selectedCustomHarness.id}:${model.value}`,
      providerId: "acp",
      providerName: selectedCustomHarness.name,
      modelId: model.value,
      reasoningLevels: [],
      defaultReasoningLevel: null,
    }));
  }, [form.harness, selectedCustomHarness]);

  const catalogModels =
    form.harness === "acp" ? acpCatalogModels : catalog?.models;
  const effectiveCatalogLoading =
    form.harness === "acp" ? false : catalogLoading;
  const modelGroups = useMemo(
    () => groupModels(catalogModels ?? []),
    [catalogModels],
  );

  // 内置主持人置顶；其余智能体保持服务端顺序。
  const orderedAgents = useMemo(
    () =>
      [...agents].sort((left, right) => {
        const leftBuiltin = left.builtinRole === "moderator" ? 1 : 0;
        const rightBuiltin = right.builtinRole === "moderator" ? 1 : 0;
        return rightBuiltin - leftBuiltin;
      }),
    [agents],
  );
  const editingAgent =
    form.editingId === null
      ? null
      : (agents.find((agent) => agent.id === form.editingId) ?? null);
  const editingBuiltin = editingAgent?.builtinRole === "moderator";
  const selectedModel =
    form.model === ""
      ? null
      : (catalogModels?.find(
          (option) =>
            option.providerId === form.providerId &&
            option.modelId === form.model,
        ) ?? null);
  const isCustomModel = form.model !== "" && selectedModel === null;
  const modelSelectValue =
    form.model === "" ? "" : (selectedModel?.key ?? CUSTOM_MODEL_VALUE);
  const customModelLabel =
    form.providerId === "" ? form.model : `${form.providerId} / ${form.model}`;

  // ACP：会话模式来自 agent 宣告的 modes（权限/思考合一的会话级模式），
  // 思考等级来自思考类 configOption（识别规则与后端一致）；两者与模型无关。
  // 内置 harness 的思考等级属于具体模型。
  const acpCapabilities = selectedCustomHarness?.capabilities ?? null;
  const acpModes = form.harness === "acp" ? acpSessionModes(acpCapabilities) : [];
  const acpReasoningChoices =
    form.harness === "acp" ? acpReasoningOption(acpCapabilities) : null;
  const reasoningLevels =
    form.harness === "acp"
      ? (acpReasoningChoices ?? []).map((choice) => choice.value)
      : (selectedModel?.reasoningLevels ?? []);
  const isCustomReasoning =
    form.reasoningLevel !== "" && !reasoningLevels.includes(form.reasoningLevel);
  const reasoningSelectValue =
    form.reasoningLevel === ""
      ? ""
      : isCustomReasoning
        ? CUSTOM_REASONING_VALUE
        : form.reasoningLevel;
  const isCustomSessionMode =
    form.sessionMode !== "" && !acpModes.some((mode) => mode.id === form.sessionMode);
  const sessionModeSelectValue =
    form.sessionMode === ""
      ? ""
      : isCustomSessionMode
        ? CUSTOM_SESSION_VALUE
        : form.sessionMode;
  const canChooseSessionMode = acpModes.length > 0;
  const canChooseReasoning =
    !effectiveCatalogLoading &&
    (form.harness === "acp"
      ? (acpReasoningChoices ?? []).length > 0
      : selectedModel !== null && reasoningLevels.length > 0);

  const modelHints: string[] = [];
  if (form.harness === "acp") {
    const capabilities = selectedCustomHarness?.capabilities ?? null;
    if (selectedCustomHarness === undefined) {
      modelHints.push("选择自定义 ACP agent 后，下拉展示其最近一次探测的模型。");
    } else if (capabilities === null) {
      modelHints.push(
        "该 agent 尚未探测模型；可在 Harness 页面探测，或留空使用 agent 默认模型。",
      );
    } else if (capabilities.models.length === 0) {
      modelHints.push("上次探测未返回模型；留空使用 agent 默认模型。");
    } else {
      const defaultModel =
        capabilities.currentModelId === null
          ? ""
          : `，agent 默认模型：${capabilities.currentModelId}`;
      modelHints.push(
        `agent 宣告了 ${capabilities.models.length} 个模型${defaultModel}；留空使用 agent 默认模型。`,
      );
    }
  } else if (catalogLoading) {
    modelHints.push("正在读取 harness 模型列表…");
  } else if (catalogError !== null) {
    modelHints.push(catalogError);
  } else if (catalog?.available === true) {
    modelHints.push(
      `共 ${catalog.models.length} 个可用模型；留空使用 harness 默认模型。`,
    );
    if (catalog.message !== null) {
      modelHints.push(catalog.message);
    }
  } else if (catalog?.available === false && catalog.message !== null) {
    modelHints.push(catalog.message);
  }
  if (isCustomModel && !catalogLoading) {
    modelHints.push("当前模型不在 harness 列表中，可直接切换或保留。");
  }
  const modelHintIds = modelHints.map(
    (_, index) => `agent-model-hint-${index}`,
  );
  const hasModelError =
    form.harness !== "acp" && !catalogLoading && catalogError !== null;

  const sessionModeHints: string[] = [];
  if (form.harness !== "acp") {
    // 内置 harness 不渲染会话模式字段。
  } else if (selectedCustomHarness === undefined) {
    sessionModeHints.push(
      "选择自定义 ACP agent 后，可从其宣告的会话模式（权限范围）中选择。",
    );
  } else if (acpModes.length === 0) {
    sessionModeHints.push(
      "该 agent 未宣告会话模式；可在 Harness 页面探测后再选，或留空跟随房间权限模式。",
    );
  } else {
    sessionModeHints.push(
      "会话模式决定运行时的权限范围（经 session/set_mode 下发），优先于房间权限映射。",
    );
  }
  const sessionModeHintIds = sessionModeHints.map(
    (_, index) => `agent-session-mode-hint-${index}`,
  );

  const reasoningHints: string[] = [];
  if (form.harness === "acp") {
    if (selectedCustomHarness === undefined) {
      reasoningHints.push(
        "选择自定义 ACP agent 后，可从其宣告的思考等级（模型推理能力）中选择。",
      );
    } else if ((acpReasoningChoices ?? []).length === 0) {
      reasoningHints.push(
        "该 agent 未宣告思考等级，运行时使用 agent 默认推理能力。",
      );
    } else {
      reasoningHints.push(
        "思考等级经 session/set_config_option 下发，与会话模式相互独立。",
      );
    }
  } else if (catalogLoading) {
    reasoningHints.push("正在读取模型的思考等级…");
  } else if (form.model === "") {
    reasoningHints.push("选择模型后可选择思考等级。");
  } else if (selectedModel === null) {
    reasoningHints.push(
      form.reasoningLevel === ""
        ? "当前模型不在 harness 列表中，无法列出思考等级。"
        : "当前模型不在 harness 列表中，已保留原有思考等级。",
    );
  } else if (reasoningLevels.length === 0) {
    reasoningHints.push("该模型未声明可选思考等级，使用模型默认。");
  } else {
    reasoningHints.push(
      selectedModel.defaultReasoningLevel === null
        ? "留空使用模型默认思考等级。"
        : `留空使用模型默认思考等级（${selectedModel.defaultReasoningLevel}）。`,
    );
  }
  if (isCustomReasoning && !catalogLoading && selectedModel !== null) {
    reasoningHints.push("当前思考等级不在列表中，保存时会保留。");
  }
  const reasoningHintIds = reasoningHints.map(
    (_, index) => `agent-reasoning-hint-${index}`,
  );

  const changeHarness = (harness: HarnessKind): void => {
    setForm((current) => ({
      ...current,
      harness,
      model: "",
      providerId: "",
      reasoningLevel: "",
      sessionMode: "",
      customHarnessId: "",
    }));
  };

  const changeCustomHarness = (value: string): void => {
    setForm((current) => ({ ...current, customHarnessId: value }));
  };

  const changeModel = (value: string): void => {
    if (value === CUSTOM_MODEL_VALUE) {
      return;
    }
    if (value === "") {
      setForm((current) => ({
        ...current,
        model: "",
        providerId: "",
        // ACP 模式与模型无关，切模型时保留；内置 harness 的等级属于模型。
        reasoningLevel:
          current.harness === "acp" ? current.reasoningLevel : "",
      }));
      return;
    }

    const option = catalogModels?.find((item) => item.key === value);
    if (option !== undefined) {
      setForm((current) => ({
        ...current,
        model: option.modelId,
        providerId: option.providerId,
        reasoningLevel:
          current.harness === "acp" ? current.reasoningLevel : "",
      }));
    }
  };

  const changeReasoningLevel = (value: string): void => {
    if (value === CUSTOM_REASONING_VALUE) {
      return;
    }
    setForm((current) => ({ ...current, reasoningLevel: value }));
  };

  const changeSessionMode = (value: string): void => {
    if (value === CUSTOM_SESSION_VALUE) {
      return;
    }
    setForm((current) => ({ ...current, sessionMode: value }));
  };

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setSaving(true);
    setError(null);

    if (form.harness === "acp" && form.customHarnessId === "") {
      setError("harness 为 acp 时必须选择一个自定义 ACP agent");
      setSaving(false);
      return;
    }

    const model = form.model.trim();
    const providerId = form.providerId.trim();
    const reasoningLevel = form.reasoningLevel.trim();
    const sessionMode = form.sessionMode.trim();
    const {
      model: _previousModel,
      providerId: _previousProviderId,
      reasoningLevel: _previousReasoningLevel,
      sessionMode: _previousSessionMode,
      ...otherConfig
    } = form.baseConfig ?? {};
    // 四个字段全空才不写模型配置；ACP 可以只选会话模式/思考等级不选模型，
    // sessionMode 与 reasoningLevel 均独立于 model 保留。
    const nextModelConfig: ModelConfig | null =
      model === "" &&
      providerId === "" &&
      reasoningLevel === "" &&
      sessionMode === ""
        ? Object.keys(otherConfig).length === 0
          ? null
          : otherConfig
        : {
            ...otherConfig,
            ...(model === "" ? {} : { model }),
            ...(providerId === "" ? {} : { providerId }),
            ...(reasoningLevel === "" ? {} : { reasoningLevel }),
            ...(sessionMode === "" ? {} : { sessionMode }),
          };
    const payload = {
      name: form.name.trim(),
      persona: form.persona.trim(),
      workspaceDir:
        form.workspaceDir.trim() === "" ? null : form.workspaceDir.trim(),
      harness: form.harness,
      modelConfig: nextModelConfig,
      ...(form.harness === "acp"
        ? {
            customHarnessId: form.customHarnessId === ""
              ? null
              : Number(form.customHarnessId),
          }
        : {}),
    };

    try {
      if (form.editingId === null) {
        await api.agents.create(payload);
      } else if (editingBuiltin) {
        // 主持人的名称/人设/工作区不可改；harness 与模型配置走同一更新接口。
        await api.agents.update(form.editingId, {
          harness: form.harness,
          modelConfig: nextModelConfig,
          ...(form.harness === "acp"
            ? { customHarnessId: Number(form.customHarnessId) }
            : {}),
        });
      } else {
        await api.agents.update(form.editingId, payload);
      }
      setForm(EMPTY_FORM);
      setFormOpen(false);
      await reload();
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setSaving(false);
    }
  };

  const startEdit = (agent: Agent) => {
    const providerId =
      typeof agent.modelConfig?.providerId === "string"
        ? agent.modelConfig.providerId
        : "";
    let reasoningLevel =
      typeof agent.modelConfig?.reasoningLevel === "string"
        ? agent.modelConfig.reasoningLevel
        : "";
    let sessionMode =
      typeof agent.modelConfig?.sessionMode === "string"
        ? agent.modelConfig.sessionMode
        : "";
    // 旧数据迁移：reasoningLevel 存的是会话模式 id（拆分前的契约）时，
    // 编辑视图把它显示为会话模式，保存后即落新契约。
    if (
      agent.harness === "acp" &&
      sessionMode === "" &&
      reasoningLevel !== ""
    ) {
      const entry = customHarnesses.find(
        (item) => String(item.id) === String(agent.customHarnessId ?? ""),
      );
      const modes = entry?.capabilities?.modes ?? [];
      if (modes.some((mode) => mode.id === reasoningLevel)) {
        sessionMode = reasoningLevel;
        reasoningLevel = "";
      }
    }
    setForm({
      editingId: agent.id,
      name: agent.name,
      persona: agent.persona,
      workspaceDir: agent.workspaceDir ?? "",
      harness: agent.harness,
      model: agent.modelConfig?.model ?? "",
      providerId,
      reasoningLevel,
      sessionMode,
      customHarnessId: agent.customHarnessId === null ? "" : String(agent.customHarnessId),
      baseConfig: agent.modelConfig,
    });
    setError(null);
    setFormOpen(true);
  };

  const closeForm = useCallback(() => {
    if (saving) {
      return;
    }
    setFormOpen(false);
    setForm(EMPTY_FORM);
    setError(null);
  }, [saving]);

  const remove = async (agent: Agent) => {
    if (!window.confirm(`确认删除智能体「${agent.name}」？`)) {
      return;
    }
    setError(null);
    try {
      await api.agents.remove(agent.id);
      if (form.editingId === agent.id) {
        setForm(EMPTY_FORM);
      }
      await reload();
    } catch (cause) {
      setError(errorMessage(cause));
    }
  };

  return (
    <section className="page">
      <PageHeader
        title="智能体"
        subtitle="创建绑定 harness 与人设的智能体，房间成员从这里选择。"
        actions={
          <>
            <div className="page-stat">
              <span className="page-stat-value">{agents.length}</span>
              <span className="page-stat-label">智能体</span>
            </div>
            <Link className="btn btn-secondary" to="/harnesses">
              管理 ACP agent
            </Link>
            <button
              type="button"
              className="btn btn-secondary"
              onClick={startCreate}
            >
              新建智能体
            </button>
          </>
        }
      />

      <Link
        className="builder-hero"
        to="/agents/builder"
        aria-label="打开智能体创建助手"
      >
        <span className="builder-hero-mark" aria-hidden="true">
          <SparkIcon />
        </span>
        <span className="builder-hero-body">
          <span className="builder-hero-title-row">
            <span className="builder-hero-title">智能体创建助手</span>
            <span className="badge badge-builtin">默认智能体 · 内置</span>
          </span>
          <span className="builder-hero-desc">
            不用填表：描述职责、风格和模型，助手会整理成草稿，确认即可创建。
          </span>
        </span>
        <span className="btn btn-primary btn-sm">开始对话 →</span>
      </Link>

      {error !== null && !formOpen && (
        <p className="banner error" role="alert">
          {error}
        </p>
      )}

      <div className="manage-main">
          {loading ? (
            <ListSkeleton variant="agent" />
          ) : agents.length === 0 ? (
            <EmptyState
              title="还没有智能体，先创建一个。"
              action={
                <button
                  type="button"
                  className="btn btn-primary btn-sm"
                  onClick={startCreate}
                >
                  新建智能体
                </button>
              }
            />
          ) : (
            <ul className="agent-list" aria-label="智能体列表">
              {orderedAgents.map((agent) => {
                const isBuiltin = agent.builtinRole === "moderator";
                return (
                <li
                  key={agent.id}
                  className={
                    form.editingId === agent.id
                      ? `agent-card is-editing${isBuiltin ? " is-builtin" : ""}`
                      : `agent-card${isBuiltin ? " is-builtin" : ""}`
                  }
                >
                  <AgentAvatar
                    id={agent.id}
                    name={agent.name}
                    size="lg"
                  />

                  <div className="agent-body">
                    <div className="agent-title-row">
                      <h3 className="agent-name">{agent.name}</h3>
                      {isBuiltin && (
                        <span className="badge badge-builtin">
                          内置 · 主持人
                        </span>
                      )}
                      <span className="badge badge-harness">
                        {agent.harness}
                      </span>
                      <span className="chip chip-mono">
                        {agent.modelConfig?.model ?? "默认"}
                        {typeof agent.modelConfig?.reasoningLevel === "string" &&
                        agent.modelConfig.reasoningLevel.trim() !== ""
                          ? ` · ${agent.modelConfig.reasoningLevel}`
                          : ""}
                      </span>
                    </div>
                    <p className="agent-persona">{agent.persona}</p>
                    {agent.workspaceDir !== null && (
                      <p className="agent-workspace" title={agent.workspaceDir}>
                        工作区：{agent.workspaceDir}
                      </p>
                    )}
                    {isBuiltin && (
                      <p className="agent-note">
                        每个房间自动加入并担任主持人；名称、人设与工作区由系统固定，
                        harness 与模型可配置。
                      </p>
                    )}
                  </div>

                  <div className="agent-actions">
                    <button
                      type="button"
                      className="btn btn-secondary btn-sm"
                      aria-label={`编辑智能体 ${agent.name}`}
                      onClick={() => startEdit(agent)}
                    >
                      编辑
                    </button>
                    {!isBuiltin && (
                      <button
                        type="button"
                        className="btn btn-danger btn-sm"
                        aria-label={`删除智能体 ${agent.name}`}
                        onClick={() => void remove(agent)}
                      >
                        删除
                      </button>
                    )}
                  </div>
                </li>
                );
              })}
            </ul>
          )}
      </div>

      {formOpen && (
        <div className="modal-layer">
          <div
            className="modal-card"
            ref={modalRef}
            role="dialog"
            aria-modal="true"
            aria-labelledby="agent-form-title"
            tabIndex={-1}
          >
            <header className="modal-head">
              <h2 id="agent-form-title" className="card-title">
                {form.editingId === null
                  ? "新建智能体"
                  : editingBuiltin
                    ? "配置内置主持人"
                    : "编辑智能体"}
              </h2>
              <button
                type="button"
                className="btn btn-ghost btn-sm"
                aria-label="关闭智能体表单"
                onClick={closeForm}
                disabled={saving}
              >
                关闭
              </button>
            </header>

            {error !== null && (
              <p className="banner error" role="alert">
                {error}
              </p>
            )}

            <form className="form-stack" onSubmit={submit}>

            {editingBuiltin && (
              <p className="field-hint">
                内置主持人的名称、人设与工作区只读；可调整 harness、模型与思考等级。
              </p>
            )}

            <div className="form-grid">
            <label className="field">
              <span className="field-label">名称</span>
              <input
                ref={nameInputRef}
                required
                value={form.name}
                disabled={editingBuiltin}
                onChange={(event) =>
                  setForm({ ...form, name: event.target.value })
                }
                placeholder="例如：规划师"
              />
            </label>

            <label className="field">
              <span className="field-label">harness</span>
              <select
                id="agent-harness"
                value={form.harness}
                onChange={(event) =>
                  changeHarness(event.target.value as HarnessKind)
                }
              >
                {HARNESS_OPTIONS.map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </select>
            </label>

            {form.harness === "acp" && (
              <label className="field">
                <span className="field-label">自定义 ACP agent</span>
                <select
                  id="agent-custom-harness"
                  value={form.customHarnessId}
                  onChange={(event) => changeCustomHarness(event.target.value)}
                  aria-label="选择自定义 ACP agent"
                >
                  <option value="">选择已注册的 ACP agent…</option>
                  {customHarnesses.map((item) => (
                    <option key={item.id} value={String(item.id)}>
                      {item.name}（{item.command}）
                    </option>
                  ))}
                </select>
                {customHarnesses.length === 0 && (
                  <span className="field-hint">
                    注册表为空：请前往 <Link to="/harnesses">Harness 页面</Link>{" "}
                    注册。
                  </span>
                )}
              </label>
            )}

            <label className="field form-span-2">
              <span className="field-label">人设</span>
              <textarea
                required
                rows={3}
                value={form.persona}
                disabled={editingBuiltin}
                onChange={(event) =>
                  setForm({ ...form, persona: event.target.value })
                }
                placeholder="描述职责与表达风格"
              />
            </label>

            <label className="field form-span-2">
              <span className="field-label">工作区目录（可选）</span>
              <input
                type="text"
                value={form.workspaceDir}
                disabled={editingBuiltin}
                onChange={(event) =>
                  setForm({ ...form, workspaceDir: event.target.value })
                }
                placeholder="留空使用房间主工作区"
                aria-label="工作区目录"
              />
              <span className="field-hint">
                目录必须已存在；运行时作为该智能体的 cwd，房间与其他成员工作区作为附加目录。
              </span>
            </label>

            <div className="field">
              <label className="field-label" htmlFor="agent-model">
                模型
              </label>
              <select
                id="agent-model"
                value={modelSelectValue}
                onChange={(event) => changeModel(event.target.value)}
                disabled={effectiveCatalogLoading}
                aria-describedby={
                  modelHintIds.length === 0
                    ? undefined
                    : modelHintIds.join(" ")
                }
              >
                <option value="">
                  {form.harness === "acp"
                    ? "使用 agent 默认模型"
                    : "使用 harness 默认模型"}
                </option>
                {isCustomModel && (
                  <option value={CUSTOM_MODEL_VALUE}>
                    当前模型：{customModelLabel}（不在列表中）
                  </option>
                )}
                {modelGroups.map((group) => (
                  <optgroup key={group.providerId} label={group.providerName}>
                    {group.models.map((option) => (
                      <option key={option.key} value={option.key}>
                        {option.modelId}
                      </option>
                    ))}
                  </optgroup>
                ))}
              </select>
              {modelHints.map((hint, index) => (
                <span
                  key={modelHintIds[index]}
                  id={modelHintIds[index]}
                  className={
                    hasModelError && index === 0
                      ? "field-error"
                      : "field-hint"
                  }
                  role={
                    hasModelError && index === 0 ? "alert" : undefined
                  }
                >
                  {hint}
                </span>
              ))}
            </div>

            {form.harness === "acp" && (
              <div className="field">
                <label className="field-label" htmlFor="agent-session-mode">
                  会话模式
                </label>
                <select
                  id="agent-session-mode"
                  value={sessionModeSelectValue}
                  onChange={(event) => changeSessionMode(event.target.value)}
                  disabled={!canChooseSessionMode}
                  aria-describedby={
                    sessionModeHintIds.length === 0
                      ? undefined
                      : sessionModeHintIds.join(" ")
                  }
                >
                  <option value="">跟随房间权限模式 / agent 默认</option>
                  {isCustomSessionMode && (
                    <option value={CUSTOM_SESSION_VALUE}>
                      当前模式：{form.sessionMode}（不在列表中）
                    </option>
                  )}
                  {acpModes.map((mode) => (
                    <option key={mode.id} value={mode.id}>
                      {mode.name}
                    </option>
                  ))}
                </select>
                {sessionModeHints.map((hint, index) => (
                  <span
                    key={sessionModeHintIds[index]}
                    id={sessionModeHintIds[index]}
                    className="field-hint"
                  >
                    {hint}
                  </span>
                ))}
              </div>
            )}

            <div className="field">
              <label className="field-label" htmlFor="agent-reasoning">
                思考等级
              </label>
              <select
                id="agent-reasoning"
                value={reasoningSelectValue}
                onChange={(event) =>
                  changeReasoningLevel(event.target.value)
                }
                disabled={!canChooseReasoning}
                aria-describedby={
                  reasoningHintIds.length === 0
                    ? undefined
                    : reasoningHintIds.join(" ")
                }
              >
                <option value="">
                  {form.harness === "acp"
                    ? "使用 agent 默认思考等级"
                    : selectedModel?.defaultReasoningLevel === null ||
                        selectedModel === null
                      ? "使用模型默认"
                      : `使用模型默认（${selectedModel.defaultReasoningLevel}）`}
                </option>
                {isCustomReasoning && (
                  <option value={CUSTOM_REASONING_VALUE}>
                    当前等级：{form.reasoningLevel}（不在列表中）
                  </option>
                )}
                {form.harness === "acp"
                  ? (acpReasoningChoices ?? []).map((choice) => (
                      <option key={choice.value} value={choice.value}>
                        {choice.name}
                      </option>
                    ))
                  : reasoningLevels.map((level) => (
                      <option key={level} value={level}>
                        {level}
                      </option>
                    ))}
              </select>
              {reasoningHints.map((hint, index) => (
                <span
                  key={reasoningHintIds[index]}
                  id={reasoningHintIds[index]}
                  className="field-hint"
                >
                  {hint}
                </span>
              ))}
            </div>

            </div>

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
                onClick={closeForm}
                disabled={saving}
              >
                取消
              </button>
            </div>
            </form>
          </div>
        </div>
      )}
    </section>
  );
}
