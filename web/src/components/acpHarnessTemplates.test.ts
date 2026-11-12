import { describe, expect, it } from "vitest";
import { parseArgsText } from "./acpHarnessForm";
import {
  ACP_HARNESS_TEMPLATES,
  resolveNpxCommand,
  templateFormValues,
} from "./acpHarnessTemplates";

describe("acpHarnessTemplates", () => {
  it("提供六个模板且 id 唯一", () => {
    const ids = ACP_HARNESS_TEMPLATES.map((template) => template.id);
    expect(ids).toEqual([
      "claude-code",
      "codex",
      "zcode",
      "kimi-code",
      "qoder",
      "dsh",
    ]);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("已核实模板预填 npx 命令与参数", () => {
    const [claude, codex] = ACP_HARNESS_TEMPLATES;
    expect(claude.prefillCommand).toBe(true);
    expect(claude.args).toEqual(["-y", "@zed-industries/claude-code-acp"]);
    expect(codex.prefillCommand).toBe(true);
    expect(codex.args).toEqual(["-y", "@zed-industries/codex-acp"]);

    const values = templateFormValues(claude, "Win32");
    expect(values.name).toBe("Claude Code ACP");
    expect(values.command).toBe("npx.cmd");
    expect(values.argsText).toBe("-y\n@zed-industries/claude-code-acp");
    expect(parseArgsText(values.argsText)).toEqual([
      "-y",
      "@zed-industries/claude-code-acp",
    ]);
  });

  it("占位模板只预填名称，命令与参数留空", () => {
    const placeholders = ACP_HARNESS_TEMPLATES.filter(
      (template) => !template.prefillCommand,
    );
    expect(placeholders.map((template) => template.id)).toEqual([
      "zcode",
      "kimi-code",
      "qoder",
      "dsh",
    ]);
    for (const template of placeholders) {
      expect(template.args).toEqual([]);
      const values = templateFormValues(template, "Win32");
      expect(values.name).toBe(template.name);
      expect(values.name).not.toBe("");
      expect(values.command).toBe("");
      expect(values.argsText).toBe("");
    }
  });

  it("Windows 使用 npx.cmd，其他平台使用 npx", () => {
    expect(
      resolveNpxCommand(
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Edg/141.0.0.0",
      ),
    ).toBe("npx.cmd");
    expect(resolveNpxCommand("windows")).toBe("npx.cmd");
    expect(resolveNpxCommand("Win32")).toBe("npx.cmd");
    expect(resolveNpxCommand("MacIntel")).toBe("npx");
    expect(resolveNpxCommand("darwin")).toBe("npx");
    expect(resolveNpxCommand("Linux x86_64")).toBe("npx");
    expect(resolveNpxCommand("")).toBe("npx");
  });
});
