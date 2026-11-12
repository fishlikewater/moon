import { describe, expect, it } from "vitest";
import type { CustomHarness } from "@moon/shared";
import {
  formatArgsText,
  formatEnvText,
  harnessFormFromItem,
  parseArgsText,
  parseEnvText,
} from "./acpHarnessForm";

describe("acpHarnessForm", () => {
  it("参数按行解析并忽略空行", () => {
    expect(parseArgsText("agent.mjs\n\n --stdio \n")).toEqual([
      "agent.mjs",
      "--stdio",
    ]);
  });

  it("环境变量保留值中的等号", () => {
    const { env, errors } = parseEnvText("A=1\nB=x=y\n");
    expect(env).toEqual({ A: "1", B: "x=y" });
    expect(errors).toEqual([]);
  });

  it("缺少等号与空 key 按行号报错且不产生半成品键", () => {
    const { env, errors } = parseEnvText("A=1\nNO_EQUALS\n=2");
    expect(env).toEqual({ A: "1" });
    expect(errors).toHaveLength(2);
    expect(errors[0]).toContain("第 2 行");
    expect(errors[1]).toContain("第 3 行");
  });

  it("重复 key 后者覆盖前者", () => {
    const { env, errors } = parseEnvText("A=1\nA=2");
    expect(errors).toEqual([]);
    expect(env).toEqual({ A: "2" });
  });

  it("格式化后能往返解析", () => {
    expect(parseArgsText(formatArgsText(["a", "b c"]))).toEqual(["a", "b c"]);
    expect(parseEnvText(formatEnvText({ A: "1", B: "x=y" })).env).toEqual({
      A: "1",
      B: "x=y",
    });
  });

  it("编辑条目预填名称、命令、参数与环境变量", () => {
    const item: CustomHarness = {
      id: 7,
      name: "gemini-cli",
      command: "npx.cmd",
      args: ["--yes", "@google/gemini-cli", "--acp"],
      env: { ACP_DEBUG: "1", API_KEY: "abc=def" },
      createdAt: 1,
      agentCount: 2,
      capabilities: null,
      probedAt: null,
    };

    expect(harnessFormFromItem(item)).toEqual({
      editingId: 7,
      name: "gemini-cli",
      command: "npx.cmd",
      argsText: "--yes\n@google/gemini-cli\n--acp",
      envText: "ACP_DEBUG=1\nAPI_KEY=abc=def",
    });
    expect(harnessFormFromItem(null)).toEqual({
      editingId: null,
      name: "",
      command: "",
      argsText: "",
      envText: "",
    });
  });
});
