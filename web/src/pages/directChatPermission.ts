import type { Room } from "@moon/shared";
import type { RoomUpdateInput } from "../api/client";

export type DirectPermissionMode = Room["permissionMode"];

export interface DirectPermissionDeps {
  update: (roomId: number, patch: RoomUpdateInput) => Promise<Room>;
}

/**
 * 直聊顶栏切换权限：同模式不发请求；不同模式只提交 permissionMode，
 * 返回服务端最新房间，失败原样抛出由页面展示。
 */
export async function changeDirectPermissionMode(
  room: Room,
  mode: DirectPermissionMode,
  deps: DirectPermissionDeps,
): Promise<Room | null> {
  if (room.permissionMode === mode) {
    return null;
  }
  return deps.update(room.id, { permissionMode: mode });
}
