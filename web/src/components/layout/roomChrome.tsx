import { createContext, useContext, useMemo, useState } from "react";
import type { ReactNode } from "react";
import type { Room } from "@moon/shared";

/**
 * 房间页向应用顶栏注入的标题信息。
 *
 * 房间页本身不再渲染页内大标题，避免重复占用垂直空间。
 */
export interface RoomChrome {
  title: string;
  backTo: string;
  backLabel: string;
  meta?: string;
}

interface RoomChromeContextValue {
  chrome: RoomChrome | null;
  setChrome: (value: RoomChrome | null) => void;
}

const RoomChromeContext = createContext<RoomChromeContextValue | null>(null);

export function RoomChromeProvider({ children }: { children: ReactNode }) {
  const [chrome, setChrome] = useState<RoomChrome | null>(null);
  const value = useMemo(() => ({ chrome, setChrome }), [chrome]);
  return (
    <RoomChromeContext.Provider value={value}>
      {children}
    </RoomChromeContext.Provider>
  );
}

export function useRoomChrome(): RoomChromeContextValue {
  const value = useContext(RoomChromeContext);
  if (value === null) {
    throw new Error("useRoomChrome 必须在 RoomChromeProvider 内使用");
  }
  return value;
}

/** 顶栏标题：房间页优先，其余页面回退到导航分区名。 */
export function resolveTopbarTitle(
  chrome: RoomChrome | null,
  fallback: string,
): string {
  return chrome === null ? fallback : chrome.title;
}

/** 由房间数据生成顶栏信息；非法房间 id 不注入 chrome，避免出现「房间 #null」。 */
export function buildRoomChrome(
  room: Room | null,
  roomId: number | null,
): RoomChrome | null {
  if (roomId === null) {
    return null;
  }
  const meta =
    room === null
      ? undefined
      : room.workspaceDirs.length > 1
        ? `${room.workspaceDir} · 另有 ${room.workspaceDirs.length - 1} 个目录`
        : room.workspaceDir;
  return {
    title: room?.name ?? `房间 #${roomId}`,
    backTo: "/rooms",
    backLabel: "返回房间列表",
    meta,
  };
}
