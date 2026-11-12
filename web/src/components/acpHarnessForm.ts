//! 自定义 ACP agent 表单的文本解析：参数按行、环境变量按 KEY=VALUE。

import type { CustomHarness } from "@moon/shared";

/** 新建/编辑表单的本地状态；与注册表条目解耦，便于弹层初始化和测试。 */
export interface HarnessFormState {
  editingId: number | null;
  name: string;
  command: string;
  argsText: string;
  envText: string;
}

export function emptyHarnessForm(): HarnessFormState {
  return {
    editingId: null,
    name: "",
    command: "",
    argsText: "",
    envText: "",
  };
}

/** 编辑时把条目的参数/环境变量格式化为每行一个；新建返回空表单。 */
export function harnessFormFromItem(
  item: CustomHarness | null,
): HarnessFormState {
  if (item === null) {
    return emptyHarnessForm();
  }
  return {
    editingId: item.id,
    name: item.name,
    command: item.command,
    argsText: formatArgsText(item.args),
    envText: formatEnvText(item.env),
  };
}

/** 参数文本：每行一个参数，忽略空行，保留参数内部空格。 */
export function parseArgsText(text: string): string[] {
  return text
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line !== "");
}

/**
 * 环境变量文本：每行 `KEY=VALUE`，值可含 `=`，空行忽略。
 * 重复 key 后者覆盖前者；缺 `=` 或空 key 按原始行号报错。
 */
export function parseEnvText(text: string): {
  env: Record<string, string>;
  errors: string[];
} {
  const env: Record<string, string> = {};
  const errors: string[] = [];

  text.split(/\r?\n/).forEach((raw, index) => {
    const line = raw.trim();
    if (line === "") {
      return;
    }
    const separator = line.indexOf("=");
    if (separator < 0) {
      errors.push(`第 ${index + 1} 行缺少 "="：${line}`);
      return;
    }
    const key = line.slice(0, separator).trim();
    if (key === "") {
      errors.push(`第 ${index + 1} 行环境变量名为空`);
      return;
    }
    env[key] = line.slice(separator + 1);
  });

  return { env, errors };
}

export function formatArgsText(args: string[]): string {
  return args.join("\n");
}

export function formatEnvText(env: Record<string, string>): string {
  return Object.entries(env)
    .map(([key, value]) => `${key}=${value}`)
    .join("\n");
}
