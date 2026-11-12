//! 内建 ACP agent 模板：只预填表单，不自动创建或写入环境变量。
//!
//! Claude Code 与 Codex 的命令来自已核实的 Zed 官方 npm 适配器包；
//! ZCode、Kimi Code、Qoder、DSH 的 ACP 入口尚未核实，只作为占位标签
//! 预填名称，启动命令需用户自行填写，不编造不可运行的命令。
//! Windows 下 `npx` 需要以 `npx.cmd` 启动，否则后端 spawn 会找不到可执行文件。

export interface AcpHarnessTemplate {
  id:
    | "claude-code"
    | "codex"
    | "zcode"
    | "kimi-code"
    | "qoder"
    | "dsh";
  /** 标签上的短名称。 */
  label: string;
  /** 预填的注册表名称。 */
  name: string;
  description: string;
  /** 逐行写入表单参数的启动参数；占位模板为空数组。 */
  args: string[];
  /** true 时预填 npx 命令与参数；false 为占位模板，只预填名称。 */
  prefillCommand: boolean;
}

export const ACP_HARNESS_TEMPLATES: AcpHarnessTemplate[] = [
  {
    id: "claude-code",
    label: "Claude Code",
    name: "Claude Code ACP",
    description: "Zed 官方 Claude Code ACP 适配器，复用本机 Claude Code 登录",
    args: ["-y", "@zed-industries/claude-code-acp"],
    prefillCommand: true,
  },
  {
    id: "codex",
    label: "Codex",
    name: "Codex ACP",
    description: "Zed 官方 Codex ACP 适配器，复用本机 Codex 登录",
    args: ["-y", "@zed-industries/codex-acp"],
    prefillCommand: true,
  },
  {
    id: "zcode",
    label: "ZCode",
    name: "ZCode ACP",
    description: "ZCode 的 ACP 入口未核实：请自行填写启动命令与参数",
    args: [],
    prefillCommand: false,
  },
  {
    id: "kimi-code",
    label: "Kimi Code",
    name: "Kimi Code ACP",
    description: "Kimi Code 的 ACP 入口未核实：请自行填写启动命令与参数",
    args: [],
    prefillCommand: false,
  },
  {
    id: "qoder",
    label: "Qoder",
    name: "Qoder ACP",
    description: "Qoder 的 ACP 入口未核实：请自行填写启动命令与参数",
    args: [],
    prefillCommand: false,
  },
  {
    id: "dsh",
    label: "DSH",
    name: "DSH ACP",
    description: "DSH 的 ACP 入口未核实：请自行填写启动命令与参数",
    args: [],
    prefillCommand: false,
  },
];

/** Windows（含大小写变体）返回 npx.cmd；其他平台返回 npx。 */
export function resolveNpxCommand(platform: string): string {
  // 调用方传的是完整 navigator.userAgent，Windows 标记不一定在开头；
  // 不能只匹配 "win"，否则 darwin 也会命中。
  return /windows|win32|win64/i.test(platform) ? "npx.cmd" : "npx";
}

/** 点击模板后写入表单的字段；占位模板命令留空，环境变量保持用户当前内容。 */
export function templateFormValues(
  template: AcpHarnessTemplate,
  platform: string,
): { name: string; command: string; argsText: string } {
  if (!template.prefillCommand) {
    return { name: template.name, command: "", argsText: "" };
  }
  return {
    name: template.name,
    command: resolveNpxCommand(platform),
    argsText: template.args.join("\n"),
  };
}
