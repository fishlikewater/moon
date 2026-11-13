import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  checkForUpdate,
  getCurrentVersion,
  isNewerVersion,
  progressPercent,
  runUpdateCheck,
  runUpdateInstall,
  sanitizeUpdateNotes,
  updateErrorMessage,
  updateStatusText,
  type UpdateProgress,
} from "./updater";

const updaterMock = vi.hoisted(() => ({ check: vi.fn() }));
const processMock = vi.hoisted(() => ({ relaunch: vi.fn() }));
const appMock = vi.hoisted(() => ({ getVersion: vi.fn() }));

vi.mock("@tauri-apps/plugin-updater", () => ({ check: updaterMock.check }));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: processMock.relaunch }));
vi.mock("@tauri-apps/api/app", () => ({ getVersion: appMock.getVersion }));

function stubDesktop(): void {
  vi.stubGlobal("window", { __MOON_API_BASE__: "http://127.0.0.1:9" });
}

/** 模拟插件 `check()` 返回的 Update 对象。 */
const pluginUpdate = () => ({
  version: "0.1.2",
  date: "2026-11-13T00:00:00Z",
  body: "## 修复\n- 会话水位",
  downloadAndInstall: vi.fn(),
});

/** 检查成功后的句柄形状；保留 mock 方法供安装测试断言。 */
const pendingUpdate = () => ({
  info: {
    version: "0.1.2",
    date: "2026-11-13T00:00:00Z",
    body: "## 修复\n- 会话水位",
  },
  downloadAndInstall: vi.fn(
    async (_onProgress: (progress: UpdateProgress) => void) => undefined,
  ),
});

beforeEach(() => {
  updaterMock.check.mockReset();
  processMock.relaunch.mockReset();
  appMock.getVersion.mockReset();
  appMock.getVersion.mockResolvedValue("0.1.1");
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("isNewerVersion", () => {
  it("按数字段比较并容忍 v 前缀", () => {
    expect(isNewerVersion("0.1.2", "0.1.1")).toBe(true);
    expect(isNewerVersion("v0.2.0", "0.1.9")).toBe(true);
    expect(isNewerVersion("0.1.10", "0.1.9")).toBe(true);
    expect(isNewerVersion("0.1.1", "0.1.1")).toBe(false);
    expect(isNewerVersion("0.2", "0.1.10")).toBe(true);
    expect(isNewerVersion("0.1.0", "0.1.1")).toBe(false);
  });

  it("预发布按数字段比较，非数字版本段不宣称更新", () => {
    expect(isNewerVersion("1.0.0-beta.1", "0.9.0")).toBe(true);
    expect(isNewerVersion("1.0.0-beta.1", "1.0.0")).toBe(false);
    expect(isNewerVersion("release", "0.1.1")).toBe(false);
  });
});

describe("updateStatusText", () => {
  it("覆盖全部阶段", () => {
    expect(updateStatusText("idle")).toBe("尚未检查更新");
    expect(updateStatusText("checking")).toBe("正在检查更新…");
    expect(updateStatusText("up_to_date")).toBe("已是最新版本");
    expect(updateStatusText("available", "0.1.2")).toBe("发现新版本 v0.1.2");
    expect(updateStatusText("available")).toBe("发现新版本");
    expect(updateStatusText("downloading")).toBe("正在下载更新…");
    expect(updateStatusText("ready")).toBe("更新已下载，正在重启…");
    expect(updateStatusText("failed")).toBe("检查或安装更新失败");
  });
});

describe("progressPercent", () => {
  it("未知总量返回 null，其余夹紧并取整", () => {
    expect(progressPercent(10, null)).toBeNull();
    expect(progressPercent(10, 0)).toBeNull();
    expect(progressPercent(0, 100)).toBe(0);
    expect(progressPercent(50, 200)).toBe(25);
    expect(progressPercent(99.6, 100)).toBe(100);
    expect(progressPercent(300, 100)).toBe(100);
    expect(progressPercent(Number.NaN, 100)).toBeNull();
  });
});

describe("sanitizeUpdateNotes", () => {
  it("统一换行、去控制字符、裁剪空行并限长", () => {
    expect(sanitizeUpdateNotes("第一行\r\n第二行\r第三行")).toBe(
      "第一行\n第二行\n第三行",
    );
    expect(sanitizeUpdateNotes("a\u0000b\u0007c")).toBe("abc");
    expect(sanitizeUpdateNotes("  \n说明\n  ")).toBe("说明");
    expect(sanitizeUpdateNotes(null)).toBe("");
    expect(sanitizeUpdateNotes("长".repeat(5000))).toHaveLength(4000);
  });
});

describe("updateErrorMessage", () => {
  it("只保留首行并限长", () => {
    expect(updateErrorMessage(new Error("下载失败\n堆栈 A\n堆栈 B"))).toBe(
      "下载失败",
    );
    expect(updateErrorMessage("字符串错误")).toBe("字符串错误");
    expect(updateErrorMessage(new Error("  \n "))).toBe(
      "更新失败，请稍后重试",
    );
    expect(updateErrorMessage({ code: 1 })).toBe("更新失败，请稍后重试");
    expect(updateErrorMessage(new Error("x".repeat(300)))).toHaveLength(200);
  });
});

describe("checkForUpdate", () => {
  it("浏览器模式不加载插件并返回 null", async () => {
    await expect(checkForUpdate()).resolves.toBeNull();
    expect(updaterMock.check).not.toHaveBeenCalled();
  });

  it("桌面模式返回可下载句柄并清洗说明", async () => {
    stubDesktop();
    const update = pluginUpdate();
    updaterMock.check.mockResolvedValue(update);
    const handle = await checkForUpdate();
    expect(handle?.info).toEqual({
      version: "0.1.2",
      date: "2026-11-13T00:00:00Z",
      body: "## 修复\n- 会话水位",
    });
    expect(update.downloadAndInstall).not.toHaveBeenCalled();
  });

  it("无更新时返回 null", async () => {
    stubDesktop();
    updaterMock.check.mockResolvedValue(null);
    await expect(checkForUpdate()).resolves.toBeNull();
  });

  it("插件返回同版本或旧版本时不提示更新", async () => {
    stubDesktop();
    updaterMock.check.mockResolvedValue({
      ...pluginUpdate(),
      version: "0.1.1",
    });
    await expect(checkForUpdate()).resolves.toBeNull();
    updaterMock.check.mockResolvedValue({
      ...pluginUpdate(),
      version: "0.1.0",
    });
    await expect(checkForUpdate()).resolves.toBeNull();
  });
});

describe("runUpdateCheck", () => {
  it("区分可更新、已最新与失败", async () => {
    stubDesktop();
    updaterMock.check.mockResolvedValueOnce(null);
    await expect(runUpdateCheck()).resolves.toEqual({ status: "up_to_date" });

    updaterMock.check.mockResolvedValueOnce(pluginUpdate());
    const available = await runUpdateCheck();
    expect(available.status).toBe("available");

    updaterMock.check.mockRejectedValueOnce(new Error("网络不可用\n栈"));
    await expect(runUpdateCheck()).resolves.toEqual({
      status: "failed",
      error: "网络不可用",
    });
  });
});

describe("runUpdateInstall", () => {
  it("下载完成后重启并上报进度", async () => {
    const update = pendingUpdate();
    const progress: number[] = [];
    update.downloadAndInstall.mockImplementation(async (onProgress) => {
      onProgress({ phase: "started", percent: 0 });
      onProgress({ phase: "progress", percent: 42 });
      onProgress({ phase: "finished", percent: 100 });
    });
    const outcome = await runUpdateInstall(update, (event) => {
      progress.push(event.percent ?? -1);
    });
    expect(outcome).toEqual({ ok: true });
    expect(processMock.relaunch).toHaveBeenCalledTimes(1);
    expect(progress).toEqual([0, 42, 100]);
  });

  it("下载失败时不重启并返回清洗后的错误", async () => {
    const update = pendingUpdate();
    update.downloadAndInstall.mockRejectedValue(
      new Error("签名校验失败\n详细堆栈"),
    );
    await expect(runUpdateInstall(update, () => undefined)).resolves.toEqual({
      ok: false,
      error: "签名校验失败",
    });
    expect(processMock.relaunch).not.toHaveBeenCalled();
  });
});

describe("getCurrentVersion", () => {
  it("桌面模式读取原生版本，浏览器模式返回 null", async () => {
    await expect(getCurrentVersion()).resolves.toBeNull();
    stubDesktop();
    appMock.getVersion.mockResolvedValue("0.1.1");
    await expect(getCurrentVersion()).resolves.toBe("0.1.1");
  });
});
