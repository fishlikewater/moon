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
import type { SessionOverview } from "@moon/shared";
import { api } from "../api/client";
import SessionsPage from "./SessionsPage";

vi.mock("../api/client", () => ({
  api: {
    sessions: {
      listAll: vi.fn(),
      archive: vi.fn(),
      remove: vi.fn(),
    },
  },
  errorMessage: (cause: unknown) =>
    cause instanceof Error ? cause.message : String(cause),
}));

function session(overrides: Partial<SessionOverview> = {}): SessionOverview {
  return {
    id: 11,
    roomId: 1,
    agentId: 2,
    harness: "codex",
    harnessSessionId: "sess-abc",
    status: "active",
    createdAt: 1,
    lastUsedAt: 2,
    lastRunId: 7,
    roomName: "房间 A",
    agentName: "小助手",
    ...overrides,
  };
}

function LocationProbe() {
  const location = useLocation();
  return <div data-testid="location">{location.pathname}</div>;
}

function renderPage(sessions: SessionOverview[]) {
  vi.mocked(api.sessions.listAll).mockResolvedValue(sessions);
  vi.mocked(api.sessions.archive).mockResolvedValue(
    session({ status: "archived" }),
  );
  vi.mocked(api.sessions.remove).mockResolvedValue(undefined);
  render(
    <MemoryRouter initialEntries={["/sessions"]}>
      <SessionsPage />
      <LocationProbe />
    </MemoryRouter>,
  );
}

async function openRowMenu(roomName: string): Promise<HTMLElement> {
  const row = (await screen.findByText(roomName)).closest("tr");
  if (row === null) {
    throw new Error("会话行未渲染");
  }
  expect(row.getAttribute("aria-haspopup")).toBe("menu");
  const notPrevented = fireEvent.contextMenu(row, {
    clientX: 120,
    clientY: 160,
  });
  expect(notPrevented).toBe(false);
  return screen.findByRole("menu", { name: "会话操作" });
}

function menuLabels(menu: HTMLElement): (string | undefined)[] {
  return within(menu)
    .getAllByRole("menuitem")
    .map((item) => item.textContent?.trim());
}

describe("SessionsPage 右键菜单", () => {
  beforeEach(() => {
    vi.mocked(api.sessions.listAll).mockReset();
    vi.mocked(api.sessions.archive).mockReset();
    vi.mocked(api.sessions.remove).mockReset();
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("活动会话的菜单顺序与可见操作一致", async () => {
    renderPage([session()]);

    const menu = await openRowMenu("房间 A");

    expect(menuLabels(menu)).toEqual([
      "打开房间",
      "打开最近运行",
      "归档",
      "删除",
    ]);
    expect(within(menu).getAllByRole("separator")).toHaveLength(1);
  });

  it("已归档且无运行链接的会话没有归档与运行项", async () => {
    renderPage([
      session({ status: "archived", lastRunId: null, roomName: "房间 B" }),
    ]);

    const menu = await openRowMenu("房间 B");

    expect(menuLabels(menu)).toEqual(["打开房间", "删除"]);
    expect(within(menu).getAllByRole("separator")).toHaveLength(1);
    expect(within(menu).getAllByRole("menuitem").at(-1)?.textContent).toBe(
      "删除",
    );
  });

  it("两个条件互不依赖：活动会话无运行时仍可归档", async () => {
    renderPage([session({ lastRunId: null, roomName: "房间 C" })]);

    const menu = await openRowMenu("房间 C");

    expect(menuLabels(menu)).toEqual(["打开房间", "归档", "删除"]);
    expect(within(menu).queryByRole("menuitem", { name: "打开最近运行" })).toBeNull();
  });

  it("已归档但有运行链接时仍显示打开最近运行", async () => {
    renderPage([
      session({ status: "archived", lastRunId: 9, roomName: "房间 D" }),
    ]);

    const menu = await openRowMenu("房间 D");

    expect(menuLabels(menu)).toEqual(["打开房间", "打开最近运行", "删除"]);
    expect(within(menu).queryByRole("menuitem", { name: "归档" })).toBeNull();
  });

  it("归档与删除复用现有确认与接口调用", async () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    renderPage([session()]);

    let menu = await openRowMenu("房间 A");
    fireEvent.click(within(menu).getByRole("menuitem", { name: "归档" }));

    await waitFor(() =>
      expect(api.sessions.archive).toHaveBeenCalledWith(1, 11),
    );
    expect(confirm).toHaveBeenCalledTimes(1);

    await waitFor(() => expect(screen.queryByRole("menu")).toBeNull());
    menu = await openRowMenu("房间 A");
    fireEvent.click(within(menu).getByRole("menuitem", { name: "删除" }));

    await waitFor(() => expect(api.sessions.remove).toHaveBeenCalledWith(1, 11));
    expect(confirm).toHaveBeenCalledTimes(2);
  });

  it("打开房间与打开最近运行走路由导航", async () => {
    renderPage([session()]);

    let menu = await openRowMenu("房间 A");
    fireEvent.click(within(menu).getByRole("menuitem", { name: "打开房间" }));
    await waitFor(() =>
      expect(screen.getByTestId("location").textContent).toBe("/rooms/1"),
    );

    await waitFor(() => expect(screen.queryByRole("menu")).toBeNull());
    menu = await openRowMenu("房间 A");
    fireEvent.click(
      within(menu).getByRole("menuitem", { name: "打开最近运行" }),
    );
    await waitFor(() =>
      expect(screen.getByTestId("location").textContent).toBe(
        "/rooms/1/runs/7",
      ),
    );
  });
});
