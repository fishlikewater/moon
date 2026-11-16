/** 直聊附件上传与发送顺序：与房间页一致，失败时清理未绑定的附件。 */

import type { Message } from "@moon/shared";

/** 附件上传只需文件的最小结构，测试可直接用普通对象。 */
export interface DirectChatSendFile {
  name: string;
  size: number;
  type: string;
}

export interface DirectChatSendDeps<
  F extends DirectChatSendFile = DirectChatSendFile,
> {
  upload: (roomId: number, file: F) => Promise<{ id: number }>;
  send: (
    roomId: number,
    content: string,
    attachmentIds: number[],
    replyToId: number | null,
  ) => Promise<{ userMessage: Message }>;
  remove: (attachmentId: number) => Promise<void>;
}

/**
 * 先逐个上传附件，再发送消息（附件 id 绑定到消息）。
 *
 * 任一步失败都会删除本次已上传但未绑定的附件，避免孤儿文件；删除失败
 * 不覆盖原始错误（与房间页 `Promise.allSettled` 语义一致）。
 */
export async function uploadAndSendDirectMessage<
  F extends DirectChatSendFile,
>(
  deps: DirectChatSendDeps<F>,
  roomId: number,
  content: string,
  files: readonly F[],
): Promise<Message> {
  const uploadedIds: number[] = [];
  try {
    for (const file of files) {
      uploadedIds.push((await deps.upload(roomId, file)).id);
    }
    const { userMessage } = await deps.send(
      roomId,
      content,
      uploadedIds,
      null,
    );
    return userMessage;
  } catch (cause) {
    await Promise.allSettled(uploadedIds.map((id) => deps.remove(id)));
    throw cause;
  }
}
