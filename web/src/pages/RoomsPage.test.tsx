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
import type { Room } from "@moon/shared";
import { api } from "../api/client";
import RoomsPage from "./RoomsPage";

vi.mock("../api/client", () => ({
  api: {
    agents: { list: vi.fn() },
    rooms: { list: vi.fn(), remove: vi.fn() },
    settings: { get: vi.fn() },
  },
  errorMessage: (cause: unknown) =>
    cause instanceof Error ? cause.message : String(cause),
  ROOM_DISCUSSION_ROUNDS: { min: 1, max: 50, fallback: 5 },
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

function LocationProbe() {
  const location = useLocation();
  return <div data-testid="location">{location.pathname}</div>;
}

function renderPage(rooms: Room[]) {
  vi.mocked(api.agents.list).mockResolvedValue([]);
  vi.mocked(api.rooms.list).mockResolvedValue(rooms);
  vi.mocked(api.rooms.remove).mockResolvedValue(undefined);
  vi.mocked(api.settings.get).mockRejectedValue(new Error("skip"));
  render(
    <MemoryRouter initialEntries={["/rooms"]}>
      <RoomsPage />
      <LocationProbe />
    </MemoryRouter>,
  );
}

async function openRowMenu(roomName: string): Promise<HTMLElement> {
  const row = (await screen.findByText(roomName)).closest("li");
  if (row === null) {
    throw new Error("房间行未渲染");
  }
  expect(row.getAttribute("aria-haspopup")).toBe("menu");
  const notPrevented = fireEvent.contextMenu(row, {
    clientX: 120,
    clientY: 160,
  });
  expect(notPrevented).toBe(false);
  return screen.findByRole("menu", { name: "房间操作" });
}

function menuLabels(menu: HTMLElement): (string | undefined)[] {
  return within(menu)
    .getAllByRole("menuitem")
    .map((item) => item.textContent?.trim());
}

describe("RoomsPage 右键菜单", () => {
  beforeEach(() => {
    vi.mocked(api.agents.list).mockReset();
    vi.mocked(api.rooms.list).mockReset();
    vi.mocked(api.rooms.remove).mockReset();
    vi.mocked(api.settings.get).mockReset();
    vi.spyOn(console, "warn").mockImplementation(() => {});
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("房间行的菜单顺序与可见操作一致", async () => {
    renderPage([room()]);

    const menu = await openRowMenu("房间 A");

    expect(menuLabels(menu)).toEqual(["进入房间", "编辑", "删除"]);
    expect(within(menu).getAllByRole("separator")).toHaveLength(1);
  });

  it("进入房间走路由导航", async () => {
    renderPage([room({ id: 5 })]);

    const menu = await openRowMenu("房间 A");
    fireEvent.click(within(menu).getByRole("menuitem", { name: "进入房间" }));

    await waitFor(() =>
      expect(screen.getByTestId("location").textContent).toBe("/rooms/5"),
    );
  });

  it("编辑复用现有房间表单", async () => {
    renderPage([room()]);

    const menu = await openRowMenu("房间 A");
    fireEvent.click(within(menu).getByRole("menuitem", { name: "编辑" }));

    const dialog = await screen.findByRole("dialog");
    expect(
      within(dialog).getByRole("heading", { name: "编辑房间" }).textContent,
    ).toBe("编辑房间");
    expect(
      (within(dialog).getByDisplayValue("房间 A") as HTMLInputElement).value,
    ).toBe("房间 A");
  });

  it("删除进行中时菜单删除项禁用", async () => {
    vi.spyOn(window, "confirm").mockReturnValue(true);
    renderPage([room({ id: 3 })]);
    let resolveRemove: () => void = () => {};
    vi.mocked(api.rooms.remove).mockImplementation(
      () =>
        new Promise<void>((resolve) => {
          resolveRemove = resolve;
        }),
    );

    let menu = await openRowMenu("房间 A");
    fireEvent.click(within(menu).getByRole("menuitem", { name: "删除" }));
    await waitFor(() => expect(api.rooms.remove).toHaveBeenCalledWith(3));
    await waitFor(() => expect(screen.queryByRole("menu")).toBeNull());

    menu = await openRowMenu("房间 A");
    const deleteItem = within(menu).getByRole("menuitem", { name: "删除" });
    expect(deleteItem.getAttribute("aria-disabled")).toBe("true");
    expect((deleteItem as HTMLButtonElement).disabled).toBe(true);

    resolveRemove();
    await waitFor(() => expect(api.rooms.list).toHaveBeenCalledTimes(2));
  });

  it("删除复用确认与接口调用", async () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    renderPage([room({ id: 3 })]);

    const menu = await openRowMenu("房间 A");
    fireEvent.click(within(menu).getByRole("menuitem", { name: "删除" }));

    await waitFor(() => expect(api.rooms.remove).toHaveBeenCalledWith(3));
    expect(confirm).toHaveBeenCalledTimes(1);
  });
});
