import { useEffect, useState } from "react";
import type { Agent, Room } from "@moon/shared";
import { api } from "./api/client";

export interface NavSummary {
  agentCount: number | null;
  roomCount: number | null;
  recentRooms: Room[];
}

interface NavSummaryDeps {
  listAgents: () => Promise<Agent[]>;
  listRooms: () => Promise<Room[]>;
}

const EMPTY_SUMMARY: NavSummary = {
  agentCount: null,
  roomCount: null,
  recentRooms: [],
};

/**
 * 并发读取侧栏需要的两个列表；任一失败只影响对应区块。
 *
 * 返回 null 表示该数据不可用，界面显示为 `—` 而不是整页错误。
 */
export async function loadNavSummary(
  deps: NavSummaryDeps = {
    listAgents: api.agents.list,
    listRooms: api.rooms.list,
  },
): Promise<NavSummary> {
  const [agents, rooms] = await Promise.allSettled([
    Promise.resolve().then(() => deps.listAgents()),
    Promise.resolve().then(() => deps.listRooms()),
  ]);

  return {
    agentCount: agents.status === "fulfilled" ? agents.value.length : null,
    roomCount: rooms.status === "fulfilled" ? rooms.value.length : null,
    recentRooms:
      rooms.status === "fulfilled"
        ? [...rooms.value]
            .sort((left, right) => right.createdAt - left.createdAt)
            .slice(0, 5)
        : [],
  };
}

/** 路由变化时刷新侧栏摘要；组件卸载后丢弃迟到结果。 */
export function useNavSummary(pathname: string): NavSummary {
  const [summary, setSummary] = useState<NavSummary>(EMPTY_SUMMARY);

  useEffect(() => {
    let cancelled = false;
    void loadNavSummary().then((next) => {
      if (!cancelled) {
        setSummary(next);
      }
    });
    return () => {
      cancelled = true;
    };
  }, [pathname]);

  return summary;
}
