import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type FormEvent,
  type KeyboardEvent,
} from "react";
import { Link } from "react-router-dom";
import type {
  Agent,
  AgentBuilderMessage,
  AgentBuilderRunInfo,
  AgentDraft,
  CustomHarness,
  HarnessKind,
  HarnessModelCatalog,
  HarnessModelOption,
  ModelConfig,
} from "@moon/shared";
import { api, errorMessage } from "../api/client";
import {
  acpDraftModels,
  acpDraftReasoningLevels,
} from "../components/agentBuilderDraft";
import {
  builderRunStatusLabel,
  builderRunSummary,
  formatBuilderCount,
  formatBuilderDuration,
} from "../components/builderRunView";
import { PageHeader } from "../components/layout/PageHeader";

const HARNESS_OPTIONS: ReadonlyArray<{ value: HarnessKind; label: string }> = [
  { value: "zcode", label: "zcode" },
  { value: "codex", label: "codex" },
  { value: "kimi-code", label: "kimi-code（暂不能运行）" },
  { value: "fake", label: "fake（仅测试）" },
  { value: "acp", label: "ACP" },
];

/** 旧草稿或助手建议的模型不在目录中时，用该值保持可见。 */
const CUSTOM_MODEL_VALUE = "__custom__";
const CUSTOM_REASONING_VALUE = "__custom_reasoning__";

const WELCOME_TEXT =
  "你好，我是智能体创建助手。告诉我你想要一个什么样的智能体：它负责什么、擅长什么、偏好用什么模型。我会把名称、人设和模型整理成右侧草稿，你确认后就能创建。";

const SUGGESTIONS = [
  "做一个严格的代码评审员",
  "做一个把需求拆成任务的技术规划师",
  "做一个写产品文案的中文编辑",
] as const;

/** 对话条目：助手回复额外携带本轮运行详情，请求时只提交 role/content。 */
interface BuilderConversationEntry extends AgentBuilderMessage {
  run?: AgentBuilderRunInfo;
}

interface DraftForm {
  name: string;
  persona: string;
  harness: HarnessKind;
  model: string;
  providerId: string;
  reasoningLevel: string;
  /** harness 为 acp 时选中的注册表条目；空字符串表示未选择。 */
  customHarnessId: string;
}

const EMPTY_DRAFT: DraftForm = {
  name: "",
  persona: "",
  harness: "zcode",
  model: "",
  providerId: "",
  reasoningLevel: "",
  customHarnessId: "",
};

const ASSISTANT_STORAGE_KEY = "moon.agentBuilder.assistant";
const LAST_CREATED_STORAGE_KEY = "moon.agentBuilder.lastCreated";

interface SavedAssistantModel {
  modelKey: string;
  reasoningLevel: string;
}

interface SavedDraftModel {
  harness: HarnessKind;
  model: string;
  providerId: string;
  reasoningLevel: string;
  /** acp 上次选择的注册表条目；旧数据没有该字段。 */
  customHarnessId?: string;
}

function readStored<T>(key: string): T | null {
  if (typeof window === "undefined") {
    return null;
  }
  try {
    const raw = window.localStorage.getItem(key);
    return raw === null ? null : (JSON.parse(raw) as T);
  } catch {
    return null;
  }
}

function writeStored(key: string, value: unknown): void {
  if (typeof window === "undefined") {
    return;
  }
  try {
    window.localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // 存储不可用时只影响“记住上次选择”，不阻断创建流程。
  }
}

/** 新建草稿默认沿用上次创建智能体时的 harness 与模型选择。 */
function defaultDraftForm(): DraftForm {
  const saved = readStored<SavedDraftModel>(LAST_CREATED_STORAGE_KEY);
  if (saved === null) {
    return { ...EMPTY_DRAFT };
  }
  return {
    ...EMPTY_DRAFT,
    harness: saved.harness,
    model: saved.model,
    providerId: saved.providerId,
    reasoningLevel: saved.reasoningLevel,
    customHarnessId: saved.customHarnessId ?? "",
  };
}

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

/**
 * 智能体创建助手：对话生成草稿，确认后调用现有智能体创建接口。
 *
 * 对话与草稿都是页面局部状态；智能体列表只作为重名检查的服务端快照。
 */
export default function AgentBuilderPage() {
  const [agents, setAgents] = useState<Agent[]>([]);
  const [conversation, setConversation] = useState<BuilderConversationEntry[]>([]);
  const [draftInput, setDraftInput] = useState("");
  const [draft, setDraft] = useState<DraftForm>(defaultDraftForm);
  const [notes, setNotes] = useState<string[]>([]);
  const [pending, setPending] = useState(false);
  const [chatError, setChatError] = useState<string | null>(null);
  const [createError, setCreateError] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [successAgent, setSuccessAgent] = useState<Agent | null>(null);

  const [assistantCatalog, setAssistantCatalog] =
    useState<HarnessModelCatalog | null>(null);
  const [assistantCatalogLoading, setAssistantCatalogLoading] = useState(true);
  const [assistantCatalogError, setAssistantCatalogError] = useState<
    string | null
  >(null);
  const [assistantModelKey, setAssistantModelKey] = useState(
    () =>
      readStored<SavedAssistantModel>(ASSISTANT_STORAGE_KEY)?.modelKey ?? "",
  );
  const [assistantReasoningLevel, setAssistantReasoningLevel] = useState(
    () =>
      readStored<SavedAssistantModel>(ASSISTANT_STORAGE_KEY)
        ?.reasoningLevel ?? "",
  );

  const [draftCatalog, setDraftCatalog] = useState<HarnessModelCatalog | null>(
    null,
  );
  const [draftCatalogLoading, setDraftCatalogLoading] = useState(true);
  const [draftCatalogError, setDraftCatalogError] = useState<string | null>(
    null,
  );
  /** 自定义 ACP agent 注册表；失败降级为空列表，创建时仍由服务端校验兜底。 */
  const [customHarnesses, setCustomHarnesses] = useState<CustomHarness[]>([]);

  const bottomRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    let cancelled = false;
    void api.agents
      .list()
      .then((list) => {
        if (!cancelled) {
          setAgents(list);
        }
      })
      .catch((cause: unknown) => {
        if (!cancelled) {
          setChatError(errorMessage(cause));
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    setAssistantCatalogLoading(true);
    setAssistantCatalogError(null);

    void api.harnesses
      .models("zcode")
      .then((catalog) => {
        if (!cancelled) {
          setAssistantCatalog(catalog);
        }
      })
      .catch((cause: unknown) => {
        if (!cancelled) {
          setAssistantCatalogError(errorMessage(cause));
        }
      })
      .finally(() => {
        if (!cancelled) {
          setAssistantCatalogLoading(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, []);

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

  useEffect(() => {
    let cancelled = false;
    setDraftCatalogLoading(true);
    setDraftCatalogError(null);
    setDraftCatalog(null);

    // ACP 没有全局模型目录：候选来自所选注册表条目的探测结果，不发目录请求。
    if (draft.harness === "acp") {
      setDraftCatalogLoading(false);
      return () => {
        cancelled = true;
      };
    }

    void api.harnesses
      .models(draft.harness)
      .then((catalog) => {
        if (!cancelled) {
          setDraftCatalog(catalog);
        }
      })
      .catch((cause: unknown) => {
        if (!cancelled) {
          setDraftCatalogError(errorMessage(cause));
        }
      })
      .finally(() => {
        if (!cancelled) {
          setDraftCatalogLoading(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [draft.harness]);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ block: "end" });
  }, [conversation, pending]);

  // 记住上次助手模型与思考等级；目录刷新后失效的选项自动清空。
  useEffect(() => {
    writeStored(ASSISTANT_STORAGE_KEY, {
      modelKey: assistantModelKey,
      reasoningLevel: assistantReasoningLevel,
    });
  }, [assistantModelKey, assistantReasoningLevel]);

  useEffect(() => {
    if (assistantCatalog === null || assistantModelKey === "") {
      return;
    }
    const model = assistantCatalog.models.find(
      (option) => option.key === assistantModelKey,
    );
    if (model === undefined) {
      setAssistantModelKey("");
      setAssistantReasoningLevel("");
      return;
    }
    if (
      assistantReasoningLevel !== "" &&
      !model.reasoningLevels.includes(assistantReasoningLevel)
    ) {
      setAssistantReasoningLevel("");
    }
  }, [assistantCatalog, assistantModelKey, assistantReasoningLevel]);

  const assistantGroups = useMemo(
    () => groupModels(assistantCatalog?.models ?? []),
    [assistantCatalog],
  );
  const assistantModel =
    assistantModelKey === ""
      ? null
      : (assistantCatalog?.models.find(
          (option) => option.key === assistantModelKey,
        ) ?? null);
  const assistantReasoningLevels = assistantModel?.reasoningLevels ?? [];
  const assistantModelConfig = useMemo<ModelConfig | null>(() => {
    if (assistantModel === null) {
      return null;
    }
    return {
      model: assistantModel.modelId,
      providerId: assistantModel.providerId,
      ...(assistantReasoningLevel === ""
        ? {}
        : { reasoningLevel: assistantReasoningLevel }),
    };
  }, [assistantModel, assistantReasoningLevel]);

  const selectedDraftCustomHarness =
    draft.customHarnessId === ""
      ? undefined
      : customHarnesses.find(
          (item) => String(item.id) === draft.customHarnessId,
        );
  const draftAcpModels = useMemo(
    () => acpDraftModels(selectedDraftCustomHarness),
    [selectedDraftCustomHarness],
  );
  // ACP 没有全局模型目录：候选来自所选注册表条目最近一次探测结果。
  const draftModels =
    draft.harness === "acp" ? draftAcpModels : draftCatalog?.models;
  const effectiveDraftCatalogLoading =
    draft.harness === "acp" ? false : draftCatalogLoading;

  const draftGroups = useMemo(
    () => groupModels(draftModels ?? []),
    [draftModels],
  );
  const selectedDraftModel =
    draft.model === ""
      ? null
      : (draftModels?.find(
          (option) =>
            option.providerId === draft.providerId &&
            option.modelId === draft.model,
        ) ?? null);
  const isCustomDraftModel = draft.model !== "" && selectedDraftModel === null;
  const draftModelSelectValue =
    draft.model === "" ? "" : (selectedDraftModel?.key ?? CUSTOM_MODEL_VALUE);
  const customDraftModelLabel =
    draft.providerId === "" ? draft.model : `${draft.providerId} / ${draft.model}`;
  const draftReasoningLevels =
    draft.harness === "acp"
      ? acpDraftReasoningLevels(selectedDraftCustomHarness)
      : (selectedDraftModel?.reasoningLevels ?? []);
  const isCustomDraftReasoning =
    draft.reasoningLevel !== "" &&
    !draftReasoningLevels.includes(draft.reasoningLevel);
  const draftReasoningSelectValue =
    draft.reasoningLevel === ""
      ? ""
      : isCustomDraftReasoning
        ? CUSTOM_REASONING_VALUE
        : draft.reasoningLevel;
  const canChooseDraftReasoning =
    !effectiveDraftCatalogLoading &&
    (draft.harness === "acp"
      ? draftReasoningLevels.length > 0
      : selectedDraftModel !== null && draftReasoningLevels.length > 0);

  const trimmedName = draft.name.trim();
  const nameTaken =
    trimmedName !== "" &&
    agents.some((agent) => agent.name === trimmedName);
  const isDraftComplete =
    trimmedName !== "" && draft.persona.trim() !== "" && !nameTaken;
  const canCreate = isDraftComplete && !creating;

  const draftModelHints: string[] = [];
  if (draft.harness === "acp") {
    if (selectedDraftCustomHarness === undefined) {
      draftModelHints.push(
        "选择自定义 ACP agent 后，下拉展示其最近一次探测的模型。",
      );
    } else if (draftAcpModels.length === 0) {
      draftModelHints.push(
        "该 ACP agent 尚未探测到模型；留空使用 agent 默认模型。",
      );
    } else {
      draftModelHints.push(
        `共 ${draftAcpModels.length} 个已探测模型；留空使用 agent 默认模型。`,
      );
    }
  } else if (effectiveDraftCatalogLoading) {
    draftModelHints.push("正在读取 harness 模型列表…");
  } else if (draftCatalogError !== null) {
    draftModelHints.push(draftCatalogError);
  } else if (draftCatalog?.available === true) {
    draftModelHints.push(
      `共 ${draftCatalog.models.length} 个可用模型；留空使用 harness 默认模型。`,
    );
    if (draftCatalog.message !== null) {
      draftModelHints.push(draftCatalog.message);
    }
  } else if (draftCatalog?.available === false && draftCatalog.message !== null) {
    draftModelHints.push(draftCatalog.message);
  }
  if (isCustomDraftModel && !effectiveDraftCatalogLoading) {
    draftModelHints.push("当前模型不在 harness 列表中，可直接切换或保留。");
  }

  const draftReasoningHints: string[] = [];
  if (draft.harness === "acp") {
    if (selectedDraftCustomHarness === undefined) {
      draftReasoningHints.push(
        "选择自定义 ACP agent 后，可从其宣告的思考等级中选择。",
      );
    } else if (draftReasoningLevels.length === 0) {
      draftReasoningHints.push(
        "该 agent 未宣告思考等级，运行时使用 agent 默认推理能力。",
      );
    } else {
      draftReasoningHints.push(
        "思考等级经 session/set_config_option 下发，与模型选择相互独立。",
      );
    }
  } else if (effectiveDraftCatalogLoading) {
    draftReasoningHints.push("正在读取模型的思考等级…");
  } else if (draft.model === "") {
    draftReasoningHints.push("选择模型后可选择思考等级。");
  } else if (selectedDraftModel === null) {
    draftReasoningHints.push(
      draft.reasoningLevel === ""
        ? "当前模型不在 harness 列表中，无法列出思考等级。"
        : "当前模型不在 harness 列表中，已保留原有思考等级。",
    );
  } else if (draftReasoningLevels.length === 0) {
    draftReasoningHints.push("该模型未声明可选思考等级，使用模型默认。");
  } else {
    draftReasoningHints.push(
      selectedDraftModel.defaultReasoningLevel === null
        ? "留空使用模型默认思考等级。"
        : `留空使用模型默认思考等级（${selectedDraftModel.defaultReasoningLevel}）。`,
    );
  }
  if (isCustomDraftReasoning && !effectiveDraftCatalogLoading) {
    draftReasoningHints.push("当前思考等级不在列表中，保存时会保留。");
  }

  const applyDraft = (next: AgentDraft): void => {
    setDraft((current) => {
      // 助手没给模型时沿用当前草稿（即上次创建的选择），换 harness 才清空。
      const keepModel = next.modelConfig === null && next.harness === current.harness;
      // 注册表条目是本机选择，助手不可能知道：沿用同一个 acp harness 时保留。
      const keepCustomHarness =
        next.harness === "acp" && current.harness === "acp";
      return {
        name: next.name,
        persona: next.persona,
        harness: next.harness,
        customHarnessId: keepCustomHarness ? current.customHarnessId : "",
        model:
          next.modelConfig?.model ?? (keepModel ? current.model : ""),
        providerId:
          typeof next.modelConfig?.providerId === "string"
            ? next.modelConfig.providerId
            : keepModel
              ? current.providerId
              : "",
        reasoningLevel:
          typeof next.modelConfig?.reasoningLevel === "string"
            ? next.modelConfig.reasoningLevel
            : keepModel
              ? current.reasoningLevel
              : "",
      };
    });
  };

  const send = async (raw: string): Promise<void> => {
    const content = raw.trim();
    if (content === "" || pending) {
      return;
    }

    const nextConversation: BuilderConversationEntry[] = [
      ...conversation,
      { role: "user", content },
    ];

    setConversation(nextConversation);
    setDraftInput("");
    setPending(true);
    setChatError(null);
    setSuccessAgent(null);

    try {
      const response = await api.agentBuilder.chat({
        // 运行详情只存在于页面状态，不进入请求体。
        messages: nextConversation.map(({ role, content }) => ({
          role,
          content,
        })),
        assistantModelConfig,
      });
      setConversation([
        ...nextConversation,
        {
          role: "assistant",
          content:
            response.reply !== ""
              ? response.reply
              : "已更新右侧草稿，请确认或继续补充要求。",
          run: response.run,
        },
      ]);
      if (response.draft !== null) {
        applyDraft(response.draft);
      }
      setNotes(response.notes);
    } catch (cause) {
      setChatError(errorMessage(cause));
    } finally {
      setPending(false);
    }
  };

  const submit = (event: FormEvent<HTMLFormElement>): void => {
    event.preventDefault();
    void send(draftInput);
  };

  const handleComposerKeyDown = (
    event: KeyboardEvent<HTMLTextAreaElement>,
  ): void => {
    if (
      event.key !== "Enter" ||
      event.shiftKey ||
      event.nativeEvent.isComposing
    ) {
      return;
    }
    event.preventDefault();
    void send(draftInput);
  };

  const changeAssistantModel = (value: string): void => {
    if (value === "") {
      setAssistantModelKey("");
      setAssistantReasoningLevel("");
      return;
    }
    if (assistantCatalog?.models.some((option) => option.key === value)) {
      setAssistantModelKey(value);
      setAssistantReasoningLevel("");
    }
  };

  const changeDraftHarness = (harness: HarnessKind): void => {
    setDraft((current) => ({
      ...current,
      harness,
      model: "",
      providerId: "",
      reasoningLevel: "",
      customHarnessId: "",
    }));
    setSuccessAgent(null);
  };

  const changeDraftCustomHarness = (value: string): void => {
    setDraft((current) => ({ ...current, customHarnessId: value }));
  };

  const changeDraftModel = (value: string): void => {
    if (value === CUSTOM_MODEL_VALUE) {
      return;
    }
    if (value === "") {
      setDraft((current) => ({
        ...current,
        model: "",
        providerId: "",
        // ACP 思考等级与模型无关，切模型时保留；内置 harness 的等级属于模型。
        reasoningLevel:
          current.harness === "acp" ? current.reasoningLevel : "",
      }));
      return;
    }

    const option = draftModels?.find((item) => item.key === value);
    if (option !== undefined) {
      setDraft((current) => ({
        ...current,
        model: option.modelId,
        providerId: option.providerId,
        reasoningLevel:
          current.harness === "acp" ? current.reasoningLevel : "",
      }));
    }
  };

  const changeDraftReasoningLevel = (value: string): void => {
    if (value === CUSTOM_REASONING_VALUE) {
      return;
    }
    setDraft((current) => ({ ...current, reasoningLevel: value }));
  };

  const resetDraft = (): void => {
    setDraft(defaultDraftForm());
    setNotes([]);
    setCreateError(null);
    setSuccessAgent(null);
  };

  const createAgent = async (): Promise<void> => {
    if (!canCreate) {
      return;
    }
    if (
      draft.harness === "acp" &&
      selectedDraftCustomHarness === undefined
    ) {
      setCreateError("harness 为 acp 时必须选择一个自定义 ACP agent");
      return;
    }

    setCreating(true);
    setCreateError(null);
    setSuccessAgent(null);

    const hasModelConfig =
      draft.model !== "" ||
      draft.providerId !== "" ||
      draft.reasoningLevel !== "";
    const modelConfig: ModelConfig | null = hasModelConfig
      ? {
          ...(draft.model === "" ? {} : { model: draft.model }),
          ...(draft.providerId === "" ? {} : { providerId: draft.providerId }),
          ...(draft.reasoningLevel === ""
            ? {}
            : { reasoningLevel: draft.reasoningLevel }),
        }
      : null;

    try {
      const created = await api.agents.create({
        name: trimmedName,
        persona: draft.persona.trim(),
        harness: draft.harness,
        modelConfig,
        ...(draft.harness === "acp" &&
        selectedDraftCustomHarness !== undefined
          ? { customHarnessId: selectedDraftCustomHarness.id }
          : {}),
      });
      setAgents((previous) => [...previous, created]);
      setSuccessAgent(created);
      writeStored(LAST_CREATED_STORAGE_KEY, {
        harness: draft.harness,
        model: draft.model,
        providerId: draft.providerId,
        reasoningLevel: draft.reasoningLevel,
        customHarnessId: draft.customHarnessId,
      });
      setDraft(defaultDraftForm());
      setNotes([]);
    } catch (cause) {
      setCreateError(errorMessage(cause));
    } finally {
      setCreating(false);
    }
  };

  return (
    <section className="page builder-page page-fixed">
      <PageHeader
        className="builder-head"
        backTo="/agents"
        backLabel="智能体"
        title="智能体创建助手"
        subtitle="像聊天一样描述你要的智能体。助手会整理名称、人设与模型，确认草稿后一键创建。"
      />

      {successAgent !== null && (
        <div className="banner success" role="status">
          <span>已创建智能体「{successAgent.name}」。</span>
          <Link className="link-button" to="/agents">
            查看智能体列表 →
          </Link>
        </div>
      )}

      {chatError !== null && (
        <p className="banner error" role="alert">
          {chatError}
        </p>
      )}

      <div className="builder-layout">
        <div className="chat-panel builder-chat">
          <div className="message-list builder-messages" aria-live="polite">
            <article className="msg msg-agent">
              <span className="avatar avatar-sm" aria-hidden="true">
                助
              </span>
              <div className="msg-body">
                <header className="msg-head">
                  <strong>创建助手</strong>
                  <span className="badge badge-builtin">默认智能体</span>
                </header>
                <p className="msg-content">{WELCOME_TEXT}</p>
                {conversation.length === 0 && (
                  <div className="builder-suggestions">
                    {SUGGESTIONS.map((suggestion) => (
                      <button
                        key={suggestion}
                        type="button"
                        className="mention-chip"
                        onClick={() => void send(suggestion)}
                        disabled={pending}
                      >
                        {suggestion}
                      </button>
                    ))}
                  </div>
                )}
              </div>
            </article>

            {conversation.map((message, index) => {
              const isUser = message.role === "user";
              return (
                <article
                  key={`${message.role}-${index}`}
                  className={`msg ${isUser ? "msg-user" : "msg-agent"}`}
                >
                  <span
                    className={`avatar avatar-sm${isUser ? " avatar-user" : ""}`}
                    aria-hidden="true"
                  >
                    {isUser ? "我" : "助"}
                  </span>
                  <div className="msg-body">
                    <header className="msg-head">
                      <strong>{isUser ? "我" : "创建助手"}</strong>
                    </header>
                    <p className="msg-content">{message.content}</p>
                    {!isUser && message.run !== undefined && (
                      <details className="builder-run">
                        <summary>{builderRunSummary(message.run)}</summary>
                        <dl className="builder-run-grid">
                          <div>
                            <dt>状态</dt>
                            <dd>{builderRunStatusLabel(message.run.status)}</dd>
                          </div>
                          <div>
                            <dt>模型</dt>
                            <dd>{message.run.model ?? "harness 默认模型"}</dd>
                          </div>
                          <div>
                            <dt>耗时</dt>
                            <dd>
                              {formatBuilderDuration(message.run.durationMs) ??
                                "—"}
                            </dd>
                          </div>
                          {formatBuilderCount(message.run.tokenCount) !==
                            null && (
                            <div>
                              <dt>Token</dt>
                              <dd>
                                {formatBuilderCount(message.run.tokenCount)}
                              </dd>
                            </div>
                          )}
                          {formatBuilderCount(message.run.toolCallCount) !==
                            null && (
                            <div>
                              <dt>工具调用</dt>
                              <dd>
                                {formatBuilderCount(message.run.toolCallCount)}
                              </dd>
                            </div>
                          )}
                        </dl>
                      </details>
                    )}
                  </div>
                </article>
              );
            })}

            {pending && (
              <article className="msg msg-agent msg-streaming">
                <span className="avatar avatar-sm" aria-hidden="true">
                  助
                </span>
                <div className="msg-body">
                  <header className="msg-head">
                    <strong>创建助手</strong>
                    <span className="msg-generating">
                      <span className="pulse-dot" aria-hidden="true" />
                      正在整理草稿
                    </span>
                  </header>
                  <p className="msg-content">正在阅读你的描述，稍等…</p>
                </div>
              </article>
            )}

            <div ref={bottomRef} />
          </div>

          <form className="composer" onSubmit={submit}>
            <textarea
              value={draftInput}
              onChange={(event) => setDraftInput(event.target.value)}
              onKeyDown={handleComposerKeyDown}
              placeholder="例如：帮我创建一个负责代码评审的智能体，风格严格，优先指出风险。"
              rows={3}
              disabled={pending}
            />
            <div className="composer-actions">
              <span className="composer-hint">
                Enter 发送，Shift + Enter 换行。创建前请在右侧确认草稿。
              </span>
              <button
                type="submit"
                className="btn btn-primary"
                disabled={pending || draftInput.trim() === ""}
              >
                {pending ? "整理中…" : "发送"}
              </button>
            </div>
          </form>
        </div>

        <aside className="builder-draft" aria-label="智能体草稿">
          <div className="card builder-draft-config" aria-label="助手模型设置">
            <div className="builder-draft-head">
              <h2 className="card-title">助手模型</h2>
              <span className="muted small">用于生成草稿</span>
            </div>
            <div className="builder-assistant-config">
              <label className="field builder-config-field">
                <span className="field-label">模型</span>
                <select
                  value={assistantModelKey}
                  onChange={(event) => changeAssistantModel(event.target.value)}
                  disabled={assistantCatalogLoading}
                >
                  <option value="">ZCode 默认模型</option>
                  {assistantGroups.map((group) => (
                    <optgroup key={group.providerId} label={group.providerName}>
                      {group.models.map((option) => (
                        <option key={option.key} value={option.key}>
                          {option.modelId}
                        </option>
                      ))}
                    </optgroup>
                  ))}
                </select>
                {assistantCatalogError !== null && (
                  <span className="field-error" role="alert">
                    {assistantCatalogError}
                  </span>
                )}
              </label>

              <label className="field builder-config-field">
                <span className="field-label">思考等级</span>
                <select
                  value={assistantReasoningLevel}
                  onChange={(event) =>
                    setAssistantReasoningLevel(event.target.value)
                  }
                  disabled={
                    assistantModel === null ||
                    assistantReasoningLevels.length === 0
                  }
                >
                  <option value="">模型默认</option>
                  {assistantReasoningLevels.map((level) => (
                    <option key={level} value={level}>
                      {level}
                    </option>
                  ))}
                </select>
              </label>
            </div>
          </div>

          <div className="card form-stack">
            <div className="builder-draft-head">
              <h2 className="card-title">智能体草稿</h2>
              <span className={isDraftComplete ? "badge badge-ready" : "badge"}>
                {isDraftComplete ? "可以创建" : "待补全"}
              </span>
            </div>

            {notes.length > 0 && (
              <ul className="builder-notes" aria-label="助手提示">
                {notes.map((note, index) => (
                  <li key={`${index}-${note}`}>{note}</li>
                ))}
              </ul>
            )}

            <label className="field">
              <span className="field-label">名称</span>
              <input
                value={draft.name}
                onChange={(event) =>
                  setDraft({ ...draft, name: event.target.value })
                }
                placeholder="例如：代码评审员"
              />
              {nameTaken ? (
                <span className="field-error" role="alert">
                  名称已存在，请换一个。
                </span>
              ) : (
                <span className="field-hint">
                  助手会从对话中补全，也可以直接填写。
                </span>
              )}
            </label>

            <label className="field">
              <span className="field-label">人设</span>
              <textarea
                rows={6}
                value={draft.persona}
                onChange={(event) =>
                  setDraft({ ...draft, persona: event.target.value })
                }
                placeholder="描述职责、擅长领域、工作方式与输出偏好"
              />
              <span className="field-hint">
                会作为智能体的系统人设，在房间运行时生效。
              </span>
            </label>

            <label className="field">
              <span className="field-label">harness</span>
              <select
                value={draft.harness}
                onChange={(event) =>
                  changeDraftHarness(event.target.value as HarnessKind)
                }
              >
                {HARNESS_OPTIONS.map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </select>
            </label>

            {draft.harness === "acp" && (
              <label className="field">
                <span className="field-label">自定义 ACP agent</span>
                <select
                  value={draft.customHarnessId}
                  onChange={(event) =>
                    changeDraftCustomHarness(event.target.value)
                  }
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

            <div className="field">
              <label className="field-label" htmlFor="builder-model">
                模型
              </label>
              <select
                id="builder-model"
                value={draftModelSelectValue}
                onChange={(event) => changeDraftModel(event.target.value)}
                disabled={effectiveDraftCatalogLoading}
              >
                <option value="">
                  {draft.harness === "acp"
                    ? "使用 agent 默认模型"
                    : "使用 harness 默认模型"}
                </option>
                {isCustomDraftModel && (
                  <option value={CUSTOM_MODEL_VALUE}>
                    当前模型：{customDraftModelLabel}（不在 harness 列表中）
                  </option>
                )}
                {draftGroups.map((group) => (
                  <optgroup key={group.providerId} label={group.providerName}>
                    {group.models.map((option) => (
                      <option key={option.key} value={option.key}>
                        {option.modelId}
                      </option>
                    ))}
                  </optgroup>
                ))}
              </select>
              {draftModelHints.map((hint, index) => (
                <span
                  key={`${index}-${hint}`}
                  className={
                    draftCatalogError !== null && index === 0
                      ? "field-error"
                      : "field-hint"
                  }
                  role={draftCatalogError !== null && index === 0 ? "alert" : undefined}
                >
                  {hint}
                </span>
              ))}
            </div>

            <div className="field">
              <label className="field-label" htmlFor="builder-reasoning">
                思考等级
              </label>
              <select
                id="builder-reasoning"
                value={draftReasoningSelectValue}
                onChange={(event) =>
                  changeDraftReasoningLevel(event.target.value)
                }
                disabled={!canChooseDraftReasoning}
              >
                <option value="">
                  {draft.harness === "acp"
                    ? "使用 agent 默认"
                    : selectedDraftModel?.defaultReasoningLevel === null ||
                        selectedDraftModel === null
                      ? "使用模型默认"
                      : `使用模型默认（${selectedDraftModel.defaultReasoningLevel}）`}
                </option>
                {isCustomDraftReasoning && (
                  <option value={CUSTOM_REASONING_VALUE}>
                    当前等级：{draft.reasoningLevel}（不在列表中）
                  </option>
                )}
                {draftReasoningLevels.map((level) => (
                  <option key={level} value={level}>
                    {level}
                  </option>
                ))}
              </select>
              {draftReasoningHints.map((hint, index) => (
                <span key={`${index}-${hint}`} className="field-hint">
                  {hint}
                </span>
              ))}
            </div>

            {createError !== null && (
              <p className="banner error" role="alert">
                {createError}
              </p>
            )}

            <div className="builder-draft-actions">
              <button
                type="button"
                className="btn btn-primary"
                disabled={!canCreate}
                onClick={() => void createAgent()}
              >
                {creating ? "创建中…" : "创建智能体"}
              </button>
              <button
                type="button"
                className="btn btn-secondary"
                onClick={resetDraft}
                disabled={creating}
              >
                清空草稿
              </button>
            </div>
          </div>
        </aside>
      </div>
    </section>
  );
}
