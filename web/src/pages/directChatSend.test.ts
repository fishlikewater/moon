import { describe, expect, it, vi } from "vitest";
import type { Message } from "@moon/shared";
import { uploadAndSendDirectMessage } from "./directChatSend";

const MESSAGE: Message = {
  id: 1,
  roomId: 7,
  seq: 1,
  senderType: "user",
  senderAgentId: null,
  status: "final",
  kind: "reply",
  runId: null,
  content: "你好",
  taskId: null,
  createdAt: 1,
};

function file(name: string): { name: string; size: number; type: string } {
  return { name, size: 10, type: "text/plain" };
}

describe("uploadAndSendDirectMessage", () => {
  it("先按顺序上传全部附件，再用附件 id 发送消息", async () => {
    const upload = vi.fn(async (_roomId: number, item: { name: string }) => ({
      id: item.name === "a.txt" ? 11 : 12,
    }));
    const send = vi.fn(async () => ({ userMessage: MESSAGE }));
    const remove = vi.fn(async () => {});

    const result = await uploadAndSendDirectMessage(
      { upload, send, remove },
      7,
      "你好",
      [file("a.txt"), file("b.txt")],
    );

    expect(result).toBe(MESSAGE);
    expect(upload.mock.calls.map(([, item]) => item.name)).toEqual([
      "a.txt",
      "b.txt",
    ]);
    expect(send).toHaveBeenCalledWith(7, "你好", [11, 12], null);
    expect(remove).not.toHaveBeenCalled();
  });

  it("发送失败时清理已上传附件并抛原错误", async () => {
    const upload = vi.fn(async () => ({ id: 11 }));
    const failure = new Error("send failed");
    const send = vi.fn(async () => {
      throw failure;
    });
    const remove = vi.fn(async () => {});

    await expect(
      uploadAndSendDirectMessage({ upload, send, remove }, 7, "你好", [
        file("a.txt"),
      ]),
    ).rejects.toBe(failure);

    expect(remove).toHaveBeenCalledTimes(1);
    expect(remove).toHaveBeenCalledWith(11);
  });

  it("上传中途失败时只清理已成功的附件且不发送", async () => {
    const upload = vi.fn(async (_roomId: number, item: { name: string }) => {
      if (item.name === "b.txt") {
        throw new Error("upload failed");
      }
      return { id: 11 };
    });
    const send = vi.fn(async () => ({ userMessage: MESSAGE }));
    const remove = vi.fn(async () => {});

    await expect(
      uploadAndSendDirectMessage({ upload, send, remove }, 7, "你好", [
        file("a.txt"),
        file("b.txt"),
      ]),
    ).rejects.toThrow("upload failed");

    expect(send).not.toHaveBeenCalled();
    expect(remove).toHaveBeenCalledTimes(1);
    expect(remove).toHaveBeenCalledWith(11);
  });
});
