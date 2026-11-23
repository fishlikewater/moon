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
import { MemoryRouter } from "react-router-dom";
import type { Room, Task } from "@moon/shared";
import { api } from "../api/client";
import TasksPage from "./TasksPage";

vi.mock("../api/client", () => ({
  api: {
    rooms: { list: vi.fn() },
    agents: { list: vi.fn() },
    tasks: { list: vi.fn(), activate: vi.fn(), timeline: vi.fn() },
    orchestration: { getState: vi.fn() },
  },
  errorMessage: (cause: unknown) =>
    cause instanceof Error ? cause.message : String(cause),
}));

function room(overrides: Partial<Room> = {}): Room {
  return {
    id: 1,
    name: "房间 A",
    roomType: "group",
    workspaceDir: "C:/tmp/room-a",
    workspaceDirs: ["C:/tmp/room-a"],
    moderatorAgentId: null,
    memberIds: [],
    replyMode: "mentions",
    discussionMaxRounds: 5,
    permissionMode: "auto",
    createdAt: 1,
    lastActivityAt: null,
    ...overrides,
  };
}

function task(overrides: Partial<Task> = {}): Task {
  return {
    id: 21,
    roomId: 1,
    title: "整理发布清单",
    description: "",
    status: "backlog",
    priority: "normal",
    assigneeAgentId: null,
    dueAt: null,
    scheduledAt: null,
    plan: null,
    createdByAgentId: null,
    createdAt: 1,
    updatedAt: 1,
    ...overrides,
  };
}

function renderPage(tasks: Task[], view: "list" | "board"): void {
  window.localStorage.setItem("moon.tasks.view", view);
  vi.mocked(api.rooms.list).mockResolvedValue([room()]);
  vi.mocked(api.agents.list).mockResolvedValue([]);
  vi.mocked(api.tasks.list).mockResolvedValue(tasks);
  vi.mocked(api.tasks.activate).mockImplementation(async (id: number) => {
    const target = tasks.find((item) => item.id === id);
    if (target === undefined) {
      throw new Error("任务不存在");
    }
    return { ...target, status: "in_progress" };
  });
  vi.mocked(api.orchestration.getState).mockResolvedValue({
    roomId: 1,
    paused: false,
  });
  vi.mocked(api.tasks.timeline).mockImplementation(async (taskId: number) => ({
    taskId,
    entries: [],
  }));
  render(
    <MemoryRouter initialEntries={["/tasks"]}>
      <TasksPage />
    </MemoryRouter>,
  );
}

async function openRowMenu(
  title: string,
  container: "article" | "tr",
): Promise<HTMLElement> {
  const item = (await screen.findByText(title)).closest(container);
  if (item === null) {
    throw new Error(`任务 ${container} 未渲染`);
  }
  expect(item.getAttribute("aria-haspopup")).toBe("menu");
  const notPrevented = fireEvent.contextMenu(item, {
    clientX: 120,
    clientY: 160,
  });
  expect(notPrevented).toBe(false);
  return screen.findByRole("menu", { name: "任务操作" });
}

function menuLabels(menu: HTMLElement) {
  return within(menu)
    .getAllByRole("menuitem")
    .map((item) => item.textContent?.trim());
}

describe("TasksPage 右键菜单", () => {
  beforeEach(() => {
    vi.mocked(api.rooms.list).mockReset();
    vi.mocked(api.agents.list).mockReset();
    vi.mocked(api.tasks.list).mockReset();
    vi.mocked(api.tasks.activate).mockReset();
    vi.mocked(api.orchestration.getState).mockReset();
    vi.mocked(api.tasks.timeline).mockReset();
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
    window.localStorage.removeItem("moon.tasks.view");
  });

  it("列表视图任务行菜单为详情、编辑、启动编排", async () => {
    renderPage([task()], "list");

    const menu = await openRowMenu("整理发布清单", "tr");

    expect(menuLabels(menu)).toEqual(["详情", "编辑", "启动编排"]);
    expect(within(menu).getAllByRole("separator")).toHaveLength(1);
  });

  it("看板卡片菜单顺序与列表视图一致", async () => {
    renderPage([task()], "board");

    const menu = await openRowMenu("整理发布清单", "article");

    expect(menuLabels(menu)).toEqual(["详情", "编辑", "启动编排"]);
    expect(within(menu).getAllByRole("separator")).toHaveLength(1);
  });

  it("不可启动的任务没有启动编排且不尾随分隔符", async () => {
    renderPage([task({ status: "done" })], "list");

    const menu = await openRowMenu("整理发布清单", "tr");

    expect(menuLabels(menu)).toEqual(["详情", "编辑"]);
    expect(within(menu).queryAllByRole("separator")).toHaveLength(0);
    expect(within(menu).getAllByRole("menuitem").at(-1)?.textContent).toBe(
      "编辑",
    );
  });

  it("启动编排进行中时菜单启动项禁用", async () => {
    renderPage([task()], "list");
    let resolveActivate: (task: Task) => void = () => {};
    vi.mocked(api.tasks.activate).mockImplementation(
      () =>
        new Promise<Task>((resolve) => {
          resolveActivate = resolve;
        }),
    );

    let menu = await openRowMenu("整理发布清单", "tr");
    fireEvent.click(within(menu).getByRole("menuitem", { name: "启动编排" }));
    await waitFor(() => expect(api.tasks.activate).toHaveBeenCalledWith(21));
    await waitFor(() => expect(screen.queryByRole("menu")).toBeNull());

    menu = await openRowMenu("整理发布清单", "tr");
    const activateItem = within(menu).getByRole("menuitem", {
      name: "启动编排",
    });
    expect(activateItem.getAttribute("aria-disabled")).toBe("true");
    expect((activateItem as HTMLButtonElement).disabled).toBe(true);

    resolveActivate({ ...task(), status: "in_progress" });
  });

  it("菜单动作打开详情、编辑与启动编排", async () => {
    renderPage([task()], "list");

    let menu = await openRowMenu("整理发布清单", "tr");
    fireEvent.click(within(menu).getByRole("menuitem", { name: "详情" }));
    await screen.findByRole("dialog", { name: "任务详情" });

    fireEvent.click(screen.getByRole("button", { name: "关闭任务详情" }));
    await waitFor(() =>
      expect(screen.queryByRole("dialog", { name: "任务详情" })).toBeNull(),
    );

    menu = await openRowMenu("整理发布清单", "tr");
    fireEvent.click(within(menu).getByRole("menuitem", { name: "编辑" }));
    const form = await screen.findByRole("dialog", { name: "编辑任务" });

    fireEvent.click(within(form).getByRole("button", { name: "取消" }));
    await waitFor(() =>
      expect(screen.queryByRole("dialog", { name: "编辑任务" })).toBeNull(),
    );

    menu = await openRowMenu("整理发布清单", "tr");
    fireEvent.click(within(menu).getByRole("menuitem", { name: "启动编排" }));
    await waitFor(() => expect(api.tasks.activate).toHaveBeenCalledWith(21));
  });
});
