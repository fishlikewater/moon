// @vitest-environment jsdom
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { MemoryRouter, useLocation } from "react-router-dom";
import type { Agent } from "@moon/shared";
import { api } from "../api/client";
import AgentsPage from "./AgentsPage";

vi.mock("../api/client", () => ({
  api: {
    agents: { list: vi.fn(), remove: vi.fn() },
    apiProviders: { list: vi.fn() },
    customHarnesses: { list: vi.fn() },
    harnesses: { models: vi.fn() },
    acpRegistry: { list: vi.fn() },
  },
  errorMessage: (cause: unknown) =>
    cause instanceof Error ? cause.message : String(cause),
}));

function agent(overrides: Partial<Agent> = {}): Agent {
  return {
    id: 2,
    name: "小助手",
    persona: "回答并给出结论。",
    harness: "fake",
    customHarnessId: null,
    workspaceDir: null,
    modelConfig: null,
    builtinRole: null,
    createdAt: 1,
    ...overrides,
  };
}

function LocationProbe() {
  const location = useLocation();
  return <div data-testid="location">{location.pathname}</div>;
}

function renderPage(agents: Agent[]) {
  vi.mocked(api.agents.list).mockResolvedValue(agents);
  vi.mocked(api.agents.remove).mockResolvedValue(undefined);
  vi.mocked(api.apiProviders.list).mockResolvedValue([]);
  vi.mocked(api.customHarnesses.list).mockResolvedValue([]);
  vi.mocked(api.harnesses.models).mockRejectedValue(new Error("skip"));
  vi.mocked(api.acpRegistry.list).mockResolvedValue({
    agents: [],
    source: "snapshot",
    fetchedAt: 1,
  });
  render(
    <MemoryRouter initialEntries={["/agents"]}>
      <AgentsPage />
      <LocationProbe />
    </MemoryRouter>,
  );
}

async function openItemMenu(agentName: string): Promise<HTMLElement> {
  const item = (await screen.findByText(agentName)).closest("li");
  if (item === null) {
    throw new Error("智能体项未渲染");
  }
  expect(item.getAttribute("aria-haspopup")).toBe("menu");
  const notPrevented = fireEvent.contextMenu(item, {
    clientX: 120,
    clientY: 160,
  });
  expect(notPrevented).toBe(false);
  return screen.findByRole("menu", { name: "智能体操作" });
}

function menuLabels(menu: HTMLElement): (string | undefined)[] {
  return within(menu)
    .getAllByRole("menuitem")
    .map((item) => item.textContent?.trim());
}

describe("AgentsPage 右键菜单", () => {
  beforeEach(() => {
    vi.mocked(api.agents.list).mockReset();
    vi.mocked(api.agents.remove).mockReset();
    vi.mocked(api.apiProviders.list).mockReset();
    vi.mocked(api.customHarnesses.list).mockReset();
    vi.mocked(api.harnesses.models).mockReset();
    vi.mocked(api.acpRegistry.list).mockReset();
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("普通智能体菜单为开始对话、编辑、删除", async () => {
    renderPage([agent()]);

    const menu = await openItemMenu("小助手");

    expect(menuLabels(menu)).toEqual(["开始对话", "编辑", "删除"]);
    expect(within(menu).getAllByRole("separator")).toHaveLength(1);
  });

  it("内置智能体没有删除项且不尾随分隔符", async () => {
    renderPage([agent({ builtinRole: "moderator", name: "主持人" })]);

    const menu = await openItemMenu("主持人");

    expect(menuLabels(menu)).toEqual(["开始对话", "编辑"]);
    expect(within(menu).queryAllByRole("separator")).toHaveLength(0);
    expect(within(menu).getAllByRole("menuitem").at(-1)?.textContent).toBe(
      "编辑",
    );
  });

  it("开始对话走路由导航", async () => {
    renderPage([agent()]);

    const menu = await openItemMenu("小助手");
    fireEvent.click(within(menu).getByRole("menuitem", { name: "开始对话" }));

    await waitFor(() =>
      expect(screen.getByTestId("location").textContent).toBe("/chat/2"),
    );
  });

  it("编辑打开现有智能体表单", async () => {
    renderPage([agent()]);

    const menu = await openItemMenu("小助手");
    fireEvent.click(within(menu).getByRole("menuitem", { name: "编辑" }));

    const dialog = await screen.findByRole("dialog");
    expect(
      (within(dialog).getByDisplayValue("小助手") as HTMLInputElement).value,
    ).toBe("小助手");
    expect(
      (
        within(dialog).getByDisplayValue(
          "回答并给出结论。",
        ) as HTMLTextAreaElement
      ).value,
    ).toBe("回答并给出结论。");
  });

  it("删除复用确认与接口调用", async () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    renderPage([agent()]);

    const menu = await openItemMenu("小助手");
    fireEvent.click(within(menu).getByRole("menuitem", { name: "删除" }));

    await waitFor(() => expect(api.agents.remove).toHaveBeenCalledWith(2));
    expect(confirm).toHaveBeenCalledTimes(1);
  });
});
