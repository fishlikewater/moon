import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { AcpHarnessFormModal } from "./AcpHarnessFormModal";
import type { AcpRegistryPayload } from "../acpRegistryCatalog";
import type { AcpRegistryState } from "../useAcpRegistry";

// createPortal 需要真实 DOM 容器，静态渲染直接以弹层内容代替 portal；
// 交互行为（背景不关闭、Esc/按钮关闭）由无头浏览器冒烟覆盖。
// 组件调用点会对 document.body 求值，这里补最小 stub 仅为通过该求值。
vi.mock("react-dom", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react-dom")>();
  return {
    ...actual,
    createPortal: (children: ReactNode) => children,
  };
});

vi.stubGlobal("document", { body: {} } as unknown as Document);

const item = {
  id: 3,
  name: "Claude ACP",
  command: "npx.cmd",
  args: ["-y", "@zed-industries/claude-code-acp"],
  env: { KEY: "value" },
  createdAt: 1,
  agentCount: 0,
  capabilities: null,
  probedAt: null,
};

function readyRegistry(): AcpRegistryState {
  const payload: AcpRegistryPayload = {
    agents: [
      {
        id: "claude-acp",
        name: "Claude Code",
        version: "0.103.0",
        summary: "Anthropic Claude Code",
        npx: { package: "@agentclientprotocol/claude-agent-acp@0.103.0", args: [] },
        windowsBinary: null,
      },
      {
        id: "opencode",
        name: "OpenCode",
        version: "2.2.6",
        summary: "",
        npx: null,
        windowsBinary: { cmd: "./opencode.exe", args: ["acp"] },
      },
    ],
    source: "network",
    fetchedAt: 1,
  };
  return { status: "ready", payload };
}

const errorRegistry: AcpRegistryState = {
  status: "error",
  message: "registry offline",
};

describe("AcpHarnessFormModal 静态渲染", () => {
  it("新建：标题、关闭按钮与表单字段齐全", () => {
    const html = renderToStaticMarkup(
      <AcpHarnessFormModal
        item={null}
        registry={readyRegistry()}
        onClose={() => {}}
        onSaved={() => {}}
      />,
    );

    expect(html).toContain("新建 ACP agent");
    expect(html).toContain('aria-label="关闭 ACP agent 表单"');
    expect(html).toContain("名称");
    expect(html).toContain("启动命令");
    // registry ready：渲染预设列表，npx 条目带版本号，二进制条目标注需本机安装。
    expect(html).toContain("acp-registry-preset-list");
    expect(html).toContain("Claude Code");
    expect(html).toContain("v0.103.0");
    expect(html).toContain("需本机安装");
    expect(html).toContain('aria-label="搜索 ACP 注册表预设"');
  });

  it("编辑：标题带条目名，参数与环境变量按行回填", () => {
    const html = renderToStaticMarkup(
      <AcpHarnessFormModal
        item={item}
        registry={readyRegistry()}
        onClose={() => {}}
        onSaved={() => {}}
      />,
    );

    expect(html).toContain("编辑 ACP agent：Claude ACP");
    expect(html).toContain('value="Claude ACP"');
    expect(html).toContain('value="npx.cmd"');
    expect(html).toContain("-y");
    expect(html).toContain("@zed-industries/claude-code-acp");
    expect(html).toContain("KEY=value");
  });

  it("registry 失败：回退内置模板并保留占位标记", () => {
    const html = renderToStaticMarkup(
      <AcpHarnessFormModal
        item={null}
        registry={errorRegistry}
        onClose={() => {}}
        onSaved={() => {}}
      />,
    );

    expect(html).toContain("注册表读取失败");
    expect(html).toContain("registry offline");
    // 回退模板：占位模板保留 is-placeholder 标记。
    expect(html).toContain("harness-template");
    expect(html).toContain("is-placeholder");
  });

  it("registry 加载中：显示加载态且不渲染预设列表", () => {
    const html = renderToStaticMarkup(
      <AcpHarnessFormModal
        item={null}
        registry={{ status: "loading" }}
        onClose={() => {}}
        onSaved={() => {}}
      />,
    );

    expect(html).toContain("正在读取 ACP 注册表");
    expect(html).not.toContain("acp-registry-preset-list");
    expect(html).not.toContain("harness-template-list");
    expect(html).not.toContain("is-placeholder");
  });
});
