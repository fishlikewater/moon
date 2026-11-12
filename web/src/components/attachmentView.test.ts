import { describe, expect, it } from "vitest";
import {
  DEFAULT_ATTACHMENT_LIMITS,
  filterPendingFiles,
  formatFileSize,
  isImageAttachment,
} from "./attachmentView";

describe("formatFileSize", () => {
  it("按字节、KB、MB 分级展示", () => {
    expect(formatFileSize(0)).toBe("0 B");
    expect(formatFileSize(1023)).toBe("1023 B");
    expect(formatFileSize(1024)).toBe("1 KB");
    expect(formatFileSize(1536)).toBe("1.5 KB");
    expect(formatFileSize(25 * 1024 * 1024)).toBe("25 MB");
  });

  it("非法值退化为未知大小", () => {
    expect(formatFileSize(-1)).toBe("未知大小");
    expect(formatFileSize(Number.NaN)).toBe("未知大小");
  });
});

describe("isImageAttachment", () => {
  it("只把 image/* 视为图片", () => {
    expect(isImageAttachment("image/png")).toBe(true);
    expect(isImageAttachment("IMAGE/JPEG")).toBe(true);
    expect(isImageAttachment("text/plain")).toBe(false);
    expect(isImageAttachment("application/octet-stream")).toBe(false);
  });
});

describe("filterPendingFiles", () => {
  const limits = { maxFileBytes: 10, maxFiles: 2 };

  it("接受数量、大小都合法的文件", () => {
    const { accepted, errors } = filterPendingFiles(0, [
      { name: "a.txt", size: 5, type: "text/plain" },
      { name: "b.png", size: 10, type: "image/png" },
    ], limits);
    expect(accepted.map((file) => file.name)).toEqual(["a.txt", "b.png"]);
    expect(errors).toEqual([]);
  });

  it("拒绝空文件与超限文件，并保留可接受项", () => {
    const { accepted, errors } = filterPendingFiles(0, [
      { name: "empty.txt", size: 0, type: "text/plain" },
      { name: "big.bin", size: 11, type: "application/octet-stream" },
      { name: "ok.txt", size: 3, type: "text/plain" },
    ], limits);
    expect(accepted.map((file) => file.name)).toEqual(["ok.txt"]);
    expect(errors).toHaveLength(2);
    expect(errors[0]).toContain("空文件");
    expect(errors[1]).toContain("超过单文件上限");
  });

  it("数量上限按已有附件累计", () => {
    const { accepted, errors } = filterPendingFiles(1, [
      { name: "a.txt", size: 1, type: "text/plain" },
      { name: "b.txt", size: 1, type: "text/plain" },
    ], limits);
    expect(accepted.map((file) => file.name)).toEqual(["a.txt"]);
    expect(errors[0]).toContain("最多 2 个附件");
  });

  it("默认限制与服务端默认值一致", () => {
    expect(DEFAULT_ATTACHMENT_LIMITS.maxFileBytes).toBe(25 * 1024 * 1024);
    expect(DEFAULT_ATTACHMENT_LIMITS.maxFiles).toBe(10);
  });
});
