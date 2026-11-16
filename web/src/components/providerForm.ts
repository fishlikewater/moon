//! 模型供应商表单的本地状态与校验纯函数。
//!
//! 模型行用结构化编辑（每模型一组字段），避免自由文本解析；
//! 序列化时剔除整行全空的行，reasoningText 按中英文逗号拆分。

import type {
  ApiProvider,
  ProviderModel,
  ProviderProtocol,
} from "@moon/shared";

/** 协议下拉候选（web 本地常量；shared 只出类型）。 */
export const PROVIDER_PROTOCOL_OPTIONS: { value: ProviderProtocol; label: string }[] = [
  { value: "anthropic", label: "Anthropic 兼容（Claude Code 系）" },
  { value: "openai", label: "OpenAI 兼容" },
];

/** 输入类型 checkbox 全集。 */
export const PROVIDER_MODEL_INPUT_TYPES = ["text", "image", "video"] as const;

/** 模型行的表单状态；contextWindowText 保留原始输入便于编辑回显。 */
export interface ModelRowState {
  id: string;
  name: string;
  contextWindowText: string;
  /** 思考等级，逗号分隔文本。 */
  reasoningText: string;
  /** 输入类型 checkbox 组；空数组 = 仅文本。 */
  inputTypes: string[];
}

/** 新建/编辑供应商表单的本地状态；与注册表条目解耦。 */
export interface ProviderFormState {
  editingId: number | null;
  name: string;
  protocol: ProviderProtocol;
  baseUrl: string;
  apiKey: string;
  enabled: boolean;
  models: ModelRowState[];
}

export function emptyModelRow(): ModelRowState {
  return {
    id: "",
    name: "",
    contextWindowText: "",
    reasoningText: "",
    inputTypes: [],
  };
}

export function emptyProviderForm(): ProviderFormState {
  return {
    editingId: null,
    name: "",
    protocol: "anthropic",
    baseUrl: "",
    apiKey: "",
    enabled: true,
    models: [emptyModelRow()],
  };
}

/** 编辑时把条目映射为表单状态；新建返回空表单。 */
export function providerFormFromItem(item: ApiProvider | null): ProviderFormState {
  if (item === null) {
    return emptyProviderForm();
  }
  const models = item.models.map((model) => ({
    id: model.id,
    name: model.name,
    contextWindowText: model.contextWindow === null ? "" : String(model.contextWindow),
    reasoningText: model.reasoningLevels.join(", "),
    inputTypes: [...model.inputTypes],
  }));
  return {
    editingId: item.id,
    name: item.name,
    protocol: item.protocol,
    baseUrl: item.baseUrl,
    apiKey: item.apiKey,
    enabled: item.enabled,
    models: models.length > 0 ? models : [emptyModelRow()],
  };
}

/** 逗号（中英文）分隔文本 → 字符串数组；剔除空项。 */
export function parseCommaList(text: string): string[] {
  return text
    .split(/[,，]/)
    .map((part) => part.trim())
    .filter((part) => part !== "");
}

/** 行是否有任何输入：全空行在序列化时剔除。 */
export function isModelRowEmpty(row: ModelRowState): boolean {
  return (
    row.id.trim() === "" &&
    row.name.trim() === "" &&
    row.contextWindowText.trim() === "" &&
    row.reasoningText.trim() === "" &&
    row.inputTypes.length === 0
  );
}

/** 表单全量校验；返回错误文案数组，空数组 = 可提交。 */
export function validateProviderForm(form: ProviderFormState): string[] {
  const errors: string[] = [];
  if (form.name.trim() === "") {
    errors.push("名称不能为空");
  }
  const baseUrl = form.baseUrl.trim();
  if (baseUrl === "") {
    errors.push("Base URL 不能为空");
  } else if (!baseUrl.startsWith("http://") && !baseUrl.startsWith("https://")) {
    errors.push("Base URL 必须以 http:// 或 https:// 开头");
  }

  const seenIds = new Set<string>();
  form.models.forEach((row, index) => {
    const label = `模型 ${index + 1}`;
    if (isModelRowEmpty(row)) {
      return;
    }
    const id = row.id.trim();
    if (id === "") {
      errors.push(`${label}的 id 不能为空`);
    } else if (seenIds.has(id)) {
      errors.push(`${label}的 id「${id}」重复`);
    } else {
      seenIds.add(id);
    }
    if (row.contextWindowText.trim() !== "") {
      const parsed = Number(row.contextWindowText.trim());
      if (!Number.isInteger(parsed) || parsed <= 0) {
        errors.push(`${label}的上下文大小必须是正整数`);
      }
    }
  });
  return errors;
}

/** 表单 → 请求体；假设 validateProviderForm 已通过。 */
export function serializeProviderForm(form: ProviderFormState): {
  name: string;
  protocol: ProviderProtocol;
  baseUrl: string;
  apiKey: string;
  enabled: boolean;
  models: ProviderModel[];
} {
  const models: ProviderModel[] = [];
  for (const row of form.models) {
    if (isModelRowEmpty(row)) {
      continue;
    }
    const id = row.id.trim();
    const windowText = row.contextWindowText.trim();
    models.push({
      id,
      name: row.name.trim() === "" ? id : row.name.trim(),
      contextWindow: windowText === "" ? null : Number(windowText),
      reasoningLevels: parseCommaList(row.reasoningText),
      inputTypes: [...row.inputTypes],
    });
  }
  return {
    name: form.name.trim(),
    protocol: form.protocol,
    baseUrl: form.baseUrl.trim(),
    apiKey: form.apiKey,
    enabled: form.enabled,
    models,
  };
}
