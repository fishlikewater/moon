import { describe, expect, it } from "vitest";
import type { CustomHarness } from "@moon/shared";
import {
  buildUpdatedArgs,
  compareVersions,
  indexRegistryByPackage,
  parseNpxPackage,
  parsePackageToken,
  registryPresetValues,
  resolveBinaryCommand,
  resolveEntryUpdateState,
  type AcpRegistryAgent,
  type AcpRegistryPayload,
} from "./acpRegistryCatalog";

function agent(overrides: Partial<AcpRegistryAgent>): AcpRegistryAgent {
  return {
    id: "demo",
    name: "Demo",
    version: "1.0.0",
    summary: "",
    npx: null,
    windowsBinary: null,
    ...overrides,
  };
}

function payload(agents: AcpRegistryAgent[]): AcpRegistryPayload {
  return { agents, source: "network", fetchedAt: 1 };
}

describe("acpRegistryCatalog", () => {
  it("解析 npm 包 token：scoped、锁定与未锁版本", () => {
    expect(parsePackageToken("@a/b@1.2.3")).toEqual({
      package: "@a/b",
      pinnedVersion: "1.2.3",
    });
    expect(parsePackageToken("@a/b")).toEqual({
      package: "@a/b",
      pinnedVersion: null,
    });
    expect(parsePackageToken("pkg@2.6.0")).toEqual({
      package: "pkg",
      pinnedVersion: "2.6.0",
    });
    expect(parsePackageToken("pkg")).toEqual({
      package: "pkg",
      pinnedVersion: null,
    });
    expect(parsePackageToken("--stdio")).toBeNull();
    expect(parsePackageToken("@=bad")).toBeNull();
  });

  it("npx 类命令解析包并跳过旗标；非 npx 命令返回 null", () => {
    expect(parseNpxPackage("npx.cmd", ["-y", "@a/b@1.0.0", "--acp"])).toEqual({
      package: "@a/b",
      pinnedVersion: "1.0.0",
    });
    expect(parseNpxPackage("npx", ["--yes", "demo-cli", "acp"])).toEqual({
      package: "demo-cli",
      pinnedVersion: null,
    });
    expect(parseNpxPackage("npx.cmd", ["-y"])).toBeNull();
    expect(parseNpxPackage("node", ["agent.mjs"])).toBeNull();
  });

  it("版本比较：数值段比较、缺段补 0、非数字段返回 null", () => {
    expect(compareVersions("0.103.0", "0.104.0")).toBe(-1);
    expect(compareVersions("2.6.0", "0.99.9")).toBe(1);
    expect(compareVersions("1.0", "1.0.0")).toBe(0);
    expect(compareVersions("1.0.0-beta", "1.0.0")).toBeNull();
  });

  it("参数改写只替换包 token，其余原样保留", () => {
    expect(buildUpdatedArgs(["-y", "@a/b@1.0.0", "--acp"], "@a/b@2.0.0")).toEqual([
      "-y",
      "@a/b@2.0.0",
      "--acp",
    ]);
    expect(buildUpdatedArgs(["--acp"], "@a/b@2.0.0")).toBeNull();
  });

  it("二进制命令解析为 PATH 裸名", () => {
    expect(resolveBinaryCommand({ cmd: "./opencode.exe", args: [] })).toBe(
      "opencode",
    );
    expect(
      resolveBinaryCommand({ cmd: "dist-package\\cursor-agent.cmd", args: [] }),
    ).toBe("cursor-agent");
    expect(resolveBinaryCommand({ cmd: "./goose", args: [] })).toBe("goose");
  });

  it("registry 预设：npx 条目预填锁版本命令，二进制条目预填裸名", () => {
    const npxAgent = agent({
      id: "claude-acp",
      name: "Claude Code",
      version: "0.103.0",
      npx: { package: "@agentclientprotocol/claude-agent-acp@0.103.0", args: [] },
    });
    expect(registryPresetValues(npxAgent, "Windows NT 10.0")).toEqual({
      name: "Claude Code",
      command: "npx.cmd",
      argsText: "-y\n@agentclientprotocol/claude-agent-acp@0.103.0",
    });
    expect(registryPresetValues(npxAgent, "Macintosh").command).toBe("npx");

    const binaryAgent = agent({
      id: "opencode",
      name: "OpenCode",
      version: "2.2.6",
      windowsBinary: { cmd: "./opencode.exe", args: ["acp"] },
    });
    expect(registryPresetValues(binaryAgent, "Windows NT 10.0")).toEqual({
      name: "OpenCode",
      command: "opencode",
      argsText: "acp",
    });
  });

  it("条目版本状态：可更新 / 已是最新 / 未锁版本 / 未知包 / 非 npx", () => {
    const registry = indexRegistryByPackage(
      payload([
        agent({
          id: "claude-acp",
          version: "0.103.0",
          npx: { package: "@agentclientprotocol/claude-agent-acp@0.103.0", args: [] },
        }),
      ]),
    );

    const outdated: Pick<CustomHarness, "command" | "args"> = {
      command: "npx.cmd",
      args: ["-y", "@agentclientprotocol/claude-agent-acp@0.102.0"],
    };
    expect(resolveEntryUpdateState(outdated, registry)).toEqual({
      kind: "update-available",
      currentVersion: "0.102.0",
      latestVersion: "0.103.0",
      updatedArgs: ["-y", "@agentclientprotocol/claude-agent-acp@0.103.0"],
    });

    const current: Pick<CustomHarness, "command" | "args"> = {
      command: "npx.cmd",
      args: ["-y", "@agentclientprotocol/claude-agent-acp@0.103.0"],
    };
    expect(resolveEntryUpdateState(current, registry)).toEqual({
      kind: "up-to-date",
      version: "0.103.0",
    });

    const unpinned: Pick<CustomHarness, "command" | "args"> = {
      command: "npx.cmd",
      args: ["-y", "@agentclientprotocol/claude-agent-acp"],
    };
    expect(resolveEntryUpdateState(unpinned, registry)).toEqual({
      kind: "unpinned",
      latestVersion: "0.103.0",
    });

    const unknown: Pick<CustomHarness, "command" | "args"> = {
      command: "npx.cmd",
      args: ["-y", "@zed-industries/claude-code-acp"],
    };
    expect(resolveEntryUpdateState(unknown, registry).kind).toBe(
      "unknown-package",
    );

    const notNpx: Pick<CustomHarness, "command" | "args"> = {
      command: "opencode",
      args: ["acp"],
    };
    expect(resolveEntryUpdateState(notNpx, registry).kind).toBe("not-npx");

    const emptyRegistry = indexRegistryByPackage(null);
    expect(resolveEntryUpdateState(current, emptyRegistry).kind).toBe(
      "no-registry",
    );
  });

  it("按包名建索引并跳过无 npx 分发的条目", () => {
    const index = indexRegistryByPackage(
      payload([
        agent({ id: "a", npx: { package: "a@1.0.0", args: [] } }),
        agent({ id: "binary-only", windowsBinary: { cmd: "./x.exe", args: [] } }),
      ]),
    );
    expect(index.has("a")).toBe(true);
    expect(index.has("binary-only")).toBe(false);
  });
});
