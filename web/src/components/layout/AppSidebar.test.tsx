import { renderToStaticMarkup } from "react-dom/server";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it } from "vitest";
import type { Agent, Room } from "@moon/shared";
import {
  AppSidebar,
  buildDirectChatEntries,
  type DirectChatEntry,
} from "./AppSidebar";
import type { NavSummary } from "../../useNavSummary";

const summary: NavSummary = {
  agentCount: 3,
  roomCount: 1,
  recentRooms: [],
};

function room(id: number, name: string): Room {
  return {
    id,
    name,
    roomType: "group",
    workspaceDir: `E:/tmp/${name}`,
    workspaceDirs: [`E:/tmp/${name}`],
    moderatorAgentId: null,
    memberIds: [1, 2],
    replyMode: "mentions",
    discussionMaxRounds: 5,
    permissionMode: "auto",
    createdAt: id,
    lastActivityAt: null,
  };
}

function render(
  path: string,
  overrides: Partial<Parameters<typeof AppSidebar>[0]> = {},
): string {
  return renderToStaticMarkup(
    <MemoryRouter initialEntries={[path]}>
      <AppSidebar
        brand={<span>Moon</span>}
        summary={summary}
        {...overrides}
      />
    </MemoryRouter>,
  );
}

function directEntry(
  overrides: Partial<DirectChatEntry> = {},
): DirectChatEntry {
  return {
    roomId: 42,
    agentId: 1,
    agentName: "小助手",
    lastActivityAt: null,
    ...overrides,
  };
}

function agent(id: number, name: string): Agent {
  return {
    id,
    name,
    persona: "",
    harness: "fake",
    customHarnessId: null,
    workspaceDir: null,
    modelConfig: null,
    builtinRole: null,
    createdAt: 1,
  };
}

describe("buildDirectChatEntries", () => {
  it("解析成员名字，过滤缺成员的直聊并按最近活动倒序", () => {
    const entries = buildDirectChatEntries(
      [
        {
          ...room(11, "与 小助手 对话"),
          roomType: "direct",
          memberIds: [1],
          lastActivityAt: 100,
        },
        {
          ...room(22, "与 审稿人 对话"),
          roomType: "direct",
          memberIds: [2],
          lastActivityAt: 300,
        },
        {
          ...room(33, "无主直聊"),
          roomType: "direct",
          memberIds: [9],
          lastActivityAt: 900,
        },
        {
          ...room(44, "空成员直聊"),
          roomType: "direct",
          memberIds: [],
          lastActivityAt: 500,
        },
      ],
      [agent(1, "小助手"), agent(2, "审稿人")],
    );

    expect(entries).toEqual([
      { roomId: 22, agentId: 2, agentName: "审稿人", lastActivityAt: 300 },
      { roomId: 11, agentId: 1, agentName: "小助手", lastActivityAt: 100 },
    ]);
  });
});

describe("AppSidebar", () => {
  it("渲染主导航并标记当前分组", () => {
    const html = render("/rooms/12");

    expect(html).toContain('href="/agents"');
    expect(html).toContain('href="/harnesses"');
    expect(html).toContain('href="/rooms"');
    expect(html).toContain('href="/tasks"');
    expect(html).toContain('href="/sessions"');
    expect(html).toContain("会话");
    const roomsAnchor =
      html.match(/<a[^>]*href="\/rooms"[^>]*>/)?.[0] ?? "";
    const agentsAnchor =
      html.match(/<a[^>]*href="\/agents"[^>]*>/)?.[0] ?? "";
    const harnessesAnchor =
      html.match(/<a[^>]*href="\/harnesses"[^>]*>/)?.[0] ?? "";
    const tasksAnchor =
      html.match(/<a[^>]*href="\/tasks"[^>]*>/)?.[0] ?? "";
    expect(roomsAnchor).toContain('aria-current="page"');
    expect(agentsAnchor).not.toContain("aria-current");
    expect(harnessesAnchor).not.toContain("aria-current");
    expect(tasksAnchor).not.toContain("aria-current");
    expect(html).toContain("app-nav-link active");
    expect(html).toContain(">3<");
    expect(html).toContain("最近房间");
  });

  it("Harness 入口高亮且不显示数量徽标", () => {
    const html = render("/harnesses");

    const harnessesAnchor =
      html.match(/<a[^>]*href="\/harnesses"[^>]*>[\s\S]*?<\/a>/)?.[0] ?? "";
    expect(harnessesAnchor).toContain('aria-current="page"');
    expect(harnessesAnchor).not.toContain("app-nav-count");
    expect(harnessesAnchor).toContain("Harness");
  });

  it("最近房间输出直达链接与成员数", () => {
    const html = render("/rooms", {
      summary: {
        agentCount: 1,
        roomCount: 2,
        recentRooms: [room(7, "搜索方案"), room(8, "发布检查")],
      },
    });

    expect(html).toContain('href="/rooms/7"');
    expect(html).toContain("搜索方案");
    expect(html).toContain("2 人");
  });

  it("计数不可用时显示破折号", () => {
    const html = render("/agents", {
      summary: { agentCount: null, roomCount: null, recentRooms: [] },
    });

    expect(html).toContain("数量不可用");
    expect((html.match(/>—<\/span>/g) ?? []).length).toBe(2);
    expect(html).toContain("房间数据不可用");
  });

  it("折叠后隐藏最近房间但保留导航", () => {
    const html = render("/agents", { collapsed: true });

    expect(html).toContain("is-collapsed");
    expect(html).not.toContain("最近房间");
    expect(html).toContain('href="/agents"');
  });

  it("有直聊时渲染对话分组与条目", () => {
    const html = render("/agents", {
      directChats: [directEntry({ lastActivityAt: Date.now() })],
    });

    expect(html).toContain("对话");
    expect(html).toContain('href="/chat/room/42"');
    expect(html).toContain("小助手");
    expect(html).toContain("刚刚");
    expect(html).toContain("最近有活动");
  });

  it("无直聊时只保留发起入口与提示，不渲染条目", () => {
    const html = render("/agents", { directChats: [] });

    expect(html).toContain("发起直聊");
    expect(html).toContain("点 ✎");
    expect(html).not.toContain("app-sidebar-direct-list");
  });

  it("折叠为图标栏时不渲染对话分组", () => {
    const html = render("/agents", {
      collapsed: true,
      directChats: [directEntry({ lastActivityAt: Date.now() })],
    });

    expect(html).not.toContain("对话");
    expect(html).not.toContain('href="/chat/room/42"');
  });
});
