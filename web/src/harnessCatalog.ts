/**
 * 内置 harness 的展示元数据与模型目录状态文案。
 *
 * 这里只保存与后端 `HarnessKind` 一一对应的静态信息；模型数量与可用性来自
 * `GET /api/harnesses/:harness/models`，由 `useHarnessCatalogs` 读取。
 */
import type { HarnessKind, HarnessModelCatalog } from "@moon/shared";

export type BuiltinHarnessStatus = "runnable" | "placeholder" | "test-only";

export type HarnessModelSource = "local-config" | "acp-registry" | "none";

export interface BuiltinHarnessInfo {
  kind: HarnessKind;
  label: string;
  status: BuiltinHarnessStatus;
  /** 运行方式的一句话说明。 */
  transport: string;
  /** 卡片上的能力标签。 */
  capabilities: readonly string[];
  /** 模型目录来源；决定卡片显示目录状态还是注册表/无模型说明。 */
  modelSource: HarnessModelSource;
}

/** 保持页面展示顺序；新增 harness 时由下面的 Record 强制补齐元数据。 */
const HARNESS_ORDER: readonly HarnessKind[] = [
  "zcode",
  "codex",
  "kimi-code",
  "acp",
  "fake",
];

const HARNESS_INFO: Record<HarnessKind, Omit<BuiltinHarnessInfo, "kind">> = {
  zcode: {
    label: "ZCode",
    status: "runnable",
    transport: "app-server / CLI",
    capabilities: ["会话复用", "附加目录", "流式回复"],
    modelSource: "local-config",
  },
  codex: {
    label: "Codex",
    status: "runnable",
    transport: "codex exec / resume",
    capabilities: ["会话复用", "流式回复"],
    modelSource: "local-config",
  },
  "kimi-code": {
    label: "Kimi Code",
    status: "placeholder",
    transport: "运行适配器未实现",
    capabilities: ["仅模型目录"],
    modelSource: "local-config",
  },
  acp: {
    label: "ACP",
    status: "runnable",
    transport: "官方 Rust SDK（stdio）",
    capabilities: ["自定义 agent", "流式回复", "工具调用", "权限模式", "会话复用"],
    modelSource: "acp-registry",
  },
  fake: {
    label: "Fake",
    status: "test-only",
    transport: "本地固定回复",
    capabilities: ["仅测试"],
    modelSource: "none",
  },
};

export const BUILTIN_HARNESSES: readonly BuiltinHarnessInfo[] = HARNESS_ORDER.map(
  (kind) => ({
    kind,
    ...HARNESS_INFO[kind],
  }),
);

/** 需要请求模型目录的内置 harness；acp 走注册表探测，fake 不使用模型。 */
export const CATALOG_READABLE_KINDS = [
  "zcode",
  "codex",
  "kimi-code",
] as const satisfies readonly HarnessKind[];

export const HARNESS_STATUS_LABELS: Record<BuiltinHarnessStatus, string> = {
  runnable: "可运行",
  placeholder: "占位",
  "test-only": "仅测试",
};

export type HarnessCatalogState =
  | { status: "loading" }
  | { status: "ready"; catalog: HarnessModelCatalog }
  | { status: "error"; message: string };

/**
 * 一张内置 harness 卡片的模型目录文案。
 *
 * `acp` 与 `fake` 不请求目录，分别提示注册表探测与无模型；其余按加载结果
 * 区分读取中、读取失败、不可用与可用数量。
 */
export function catalogStatusText(
  info: BuiltinHarnessInfo,
  state: HarnessCatalogState | undefined,
): string {
  if (info.modelSource === "none") {
    return "不使用模型";
  }
  if (info.modelSource === "acp-registry") {
    return "模型由 ACP agent 探测决定";
  }
  if (state === undefined || state.status === "loading") {
    return "正在读取模型目录…";
  }
  if (state.status === "error") {
    return `模型目录读取失败：${state.message}`;
  }
  const { catalog } = state;
  if (!catalog.available) {
    return `模型目录不可用：${catalog.message ?? "未返回原因"}`;
  }
  return `模型目录可用：${catalog.models.length} 个模型`;
}
