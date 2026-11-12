/** 附件展示与上传前校验的纯逻辑。 */

export interface AttachmentLimits {
  maxFileBytes: number;
  maxFiles: number;
}

export const DEFAULT_ATTACHMENT_LIMITS: AttachmentLimits = {
  maxFileBytes: 26_214_400,
  maxFiles: 10,
};

export interface AttachmentFileLike {
  name: string;
  size: number;
  type: string;
}

/** 人类可读大小；0 与 1023 保留字节，其余按 KB/MB 保留一位小数。 */
export function formatFileSize(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) {
    return "未知大小";
  }
  if (bytes < 1024) {
    return `${bytes} B`;
  }
  const units = ["KB", "MB", "GB"];
  let value = bytes / 1024;
  let index = 0;
  while (value >= 1024 && index < units.length - 1) {
    value /= 1024;
    index += 1;
  }
  const rounded = Math.round(value * 10) / 10;
  return `${rounded} ${units[index]}`;
}

export function isImageAttachment(mimeType: string): boolean {
  return mimeType.toLowerCase().startsWith("image/");
}

export interface FilteredAttachments<T extends AttachmentFileLike> {
  accepted: T[];
  errors: string[];
}

/** 上传前校验：数量、空文件与单文件大小；只返回可接受的条目。 */
export function filterPendingFiles<T extends AttachmentFileLike>(
  existingCount: number,
  files: readonly T[],
  limits: AttachmentLimits,
): FilteredAttachments<T> {
  const accepted: T[] = [];
  const errors: string[] = [];

  for (const file of files) {
    if (existingCount + accepted.length >= limits.maxFiles) {
      errors.push(`单条消息最多 ${limits.maxFiles} 个附件`);
      break;
    }
    if (file.size <= 0) {
      errors.push(`「${file.name}」是空文件`);
      continue;
    }
    if (file.size > limits.maxFileBytes) {
      errors.push(
        `「${file.name}」超过单文件上限（${formatFileSize(limits.maxFileBytes)}）`,
      );
      continue;
    }
    accepted.push(file);
  }

  return { accepted, errors };
}
