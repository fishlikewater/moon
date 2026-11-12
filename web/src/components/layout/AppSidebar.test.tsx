import { renderToStaticMarkup } from "react-dom/server";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it } from "vitest";
import type { Room } from "@moon/shared";
import { AppSidebar } from "./AppSidebar";
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
});
