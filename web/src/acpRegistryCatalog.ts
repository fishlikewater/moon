//! ACP registry 目录的前端纯函数：平台命令解析、包名/版本解析、参数改写
//! 与注册表条目的版本状态推导。不做网络请求，不持有 React 状态。

import type { CustomHarness } from "@moon/shared";

/** registry 目录中的一个 agent（已由后端按平台修剪）。 */
export interface AcpRegistryAgent {
  id: string;
  name: string;
  version: string;
  summary: string;
  /** npx 分发：包名（含 `@version` 锁定）与附加参数。 */
  npx: AcpRegistryNpx | null;
  /** windows-x86_64 二进制分发；供「需本机安装」类预设展示。 */
  windowsBinary: AcpRegistryBinary | null;
}

export interface AcpRegistryNpx {
  package: string;
  args: string[];
}

export interface AcpRegistryBinary {
  cmd: string;
  args: string[];
}

export type AcpRegistrySource = "network" | "cache" | "snapshot";

export interface AcpRegistryPayload {
  agents: AcpRegistryAgent[];
  source: AcpRegistrySource;
  fetchedAt: number;
}

/** registry 预设点击后写入表单的内容。 */
export interface RegistryPresetValues {
  name: string;
  command: string;
  argsText: string;
}

const NPX_COMMANDS = new Set(["npx", "npx.cmd", "npx.exe"]);

/** 无 scope 的合法 npm 包名段。 */
const PACKAGE_NAME_RE = /^[A-Za-z0-9][A-Za-z0-9._-]*$/;
/** `@scope/name` 或 `@scope/name@version`。 */
const SCOPED_PACKAGE_RE = /^@([^@/]+)\/([^@/]+)(?:@(.+))?$/;

/** 解析单个 npm 包 token；`@scope/name@ver`、`name@ver`、`name` 均可。 */
export function parsePackageToken(token: string): {
  package: string;
  pinnedVersion: string | null;
} | null {
  if (token.startsWith("@")) {
    const match = SCOPED_PACKAGE_RE.exec(token);
    if (match === null) {
      return null;
    }
    return {
      package: `@${match[1]}/${match[2]}`,
      pinnedVersion: match[3] ?? null,
    };
  }
  const separator = token.indexOf("@");
  if (separator > 0) {
    const name = token.slice(0, separator);
    if (!PACKAGE_NAME_RE.test(name) || name.includes("/")) {
      return null;
    }
    return {
      package: name,
      pinnedVersion: token.slice(separator + 1) || null,
    };
  }
  // 无 `@` 的裸 token：按 npx 语义视为未锁版本的包名。
  return PACKAGE_NAME_RE.test(token)
    ? { package: token, pinnedVersion: null }
    : null;
}

/**
 * windows-x86_64 二进制 cmd → PATH 友好裸名。
 * `./opencode.exe`、`dist-package\\cursor-agent.cmd` → `opencode`、`cursor-agent`。
 */
export function resolveBinaryCommand(binary: AcpRegistryBinary): string {
  const normalized = binary.cmd.replace(/\\/g, "/").replace(/^\.\/+/, "");
  const base = normalized.split("/").pop() ?? normalized;
  return base.replace(/\.exe$/i, "").replace(/\.cmd$/i, "");
}

/**
 * 解析 npx 类命令参数中的 npm 包：跳过 `-y`/`--yes` 等旗标后，
 * 第一个非旗标 token 按 npx 语义就是被运行的包。
 */
export function parseNpxPackage(
  command: string,
  args: string[],
): { package: string; pinnedVersion: string | null } | null {
  if (!NPX_COMMANDS.has(command.trim().toLowerCase())) {
    return null;
  }
  for (const arg of args) {
    if (arg.startsWith("-")) {
      continue;
    }
    return parsePackageToken(arg);
  }
  return null;
}

/**
 * 点分数字版本比较；返回 -1/0/1，含非数字段时返回 null
 * （调用方只展示信息，不做升级判定）。
 */
export function compareVersions(a: string, b: string): number | null {
  const left = a.split(".");
  const right = b.split(".");
  const length = Math.max(left.length, right.length);
  for (let index = 0; index < length; index += 1) {
    const a = Number(left[index] ?? "0");
    const b = Number(right[index] ?? "0");
    if (!Number.isFinite(a) || !Number.isFinite(b)) {
      return null;
    }
    if (a !== b) {
      return a < b ? -1 : 1;
    }
  }
  return 0;
}

/**
 * 把 args 中第一个包 token 替换为最新包规格（`pkg@version`），
 * 其余参数原样保留；找不到包 token 时返回 null。
 */
export function buildUpdatedArgs(
  args: string[],
  latestPackageSpec: string,
): string[] | null {
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    if (arg === undefined || arg.startsWith("-")) {
      continue;
    }
    if (parsePackageToken(arg) !== null) {
      const next = args.slice();
      next[index] = latestPackageSpec;
      return next;
    }
  }
  return null;
}

/** 注册表条目的版本状态；UI 据此渲染徽标与更新按钮。 */
export type EntryUpdateState =
  | { kind: "not-npx" }
  | { kind: "no-package" }
  | { kind: "no-registry" }
  | { kind: "unknown-package" }
  | { kind: "unpinned"; latestVersion: string }
  | { kind: "incomparable"; currentVersion: string; latestVersion: string }
  | { kind: "up-to-date"; version: string }
  | {
      kind: "update-available";
      currentVersion: string;
      latestVersion: string;
      updatedArgs: string[];
    };

/** registry 按「包名 → agent」索引，供条目匹配。 */
export function indexRegistryByPackage(
  payload: AcpRegistryPayload | null,
): Map<string, AcpRegistryAgent> {
  const index = new Map<string, AcpRegistryAgent>();
  if (payload === null) {
    return index;
  }
  for (const agent of payload.agents) {
    if (agent.npx === null) {
      continue;
    }
    const parsed = parsePackageToken(agent.npx.package);
    if (parsed !== null && !index.has(parsed.package)) {
      index.set(parsed.package, agent);
    }
  }
  return index;
}

/** 推导一个注册表条目的版本状态；registry 未加载时返回 no-registry。 */
export function resolveEntryUpdateState(
  entry: Pick<CustomHarness, "command" | "args">,
  registryByPackage: Map<string, AcpRegistryAgent>,
): EntryUpdateState {
  const local = parseNpxPackage(entry.command, entry.args);
  if (local === null) {
    return NPX_COMMANDS.has(entry.command.trim().toLowerCase())
      ? { kind: "no-package" }
      : { kind: "not-npx" };
  }
  if (registryByPackage.size === 0) {
    return { kind: "no-registry" };
  }
  const agent = registryByPackage.get(local.package);
  if (agent === undefined) {
    return { kind: "unknown-package" };
  }
  if (local.pinnedVersion === null) {
    return { kind: "unpinned", latestVersion: agent.version };
  }
  const comparison = compareVersions(local.pinnedVersion, agent.version);
  if (comparison === null) {
    return {
      kind: "incomparable",
      currentVersion: local.pinnedVersion,
      latestVersion: agent.version,
    };
  }
  if (comparison >= 0) {
    return { kind: "up-to-date", version: local.pinnedVersion };
  }
  const updatedArgs = buildUpdatedArgs(entry.args, agent.npx?.package ?? "");
  if (updatedArgs === null) {
    return { kind: "no-package" };
  }
  return {
    kind: "update-available",
    currentVersion: local.pinnedVersion,
    latestVersion: agent.version,
    updatedArgs,
  };
}

/**
 * registry 预设点击后写入表单的值。
 * npx 条目：`npx.cmd`（Windows）/ `npx` + `-y <pkg>@<version>` + registry args。
 * 二进制条目：PATH 裸名 + registry args（需本机已安装）。
 */
export function registryPresetValues(
  agent: AcpRegistryAgent,
  platform: string,
): RegistryPresetValues {
  const isWindows = /windows|win32|win64/i.test(platform);
  if (agent.npx !== null) {
    return {
      name: agent.name,
      command: isWindows ? "npx.cmd" : "npx",
      argsText: ["-y", agent.npx.package, ...agent.npx.args].join("\n"),
    };
  }
  if (agent.windowsBinary !== null) {
    return {
      name: agent.name,
      command: resolveBinaryCommand(agent.windowsBinary),
      argsText: agent.windowsBinary.args.join("\n"),
    };
  }
  // 两者都缺（理论上不会出现）：只预填名称。
  return { name: agent.name, command: "", argsText: "" };
}
