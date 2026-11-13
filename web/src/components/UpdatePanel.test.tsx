import { renderToStaticMarkup } from "react-dom/server";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it, vi } from "vitest";
import { UpdateBannerView } from "./UpdateBanner";
import {
  initialUpdatePanelState,
  updatePanelReducer,
  UpdatePanelView,
  type UpdatePanelViewProps,
} from "./UpdatePanel";
import type { PendingUpdate } from "../updater";

const pending: PendingUpdate = {
  info: {
    version: "0.1.2",
    date: "2026-11-13T00:00:00Z",
    body: "## 修复\n- 会话水位",
  },
  downloadAndInstall: async () => undefined,
};

function renderPanel(overrides: Partial<UpdatePanelViewProps> = {}): string {
  return renderToStaticMarkup(
    <UpdatePanelView
      desktop
      phase="idle"
      currentVersion="0.1.1"
      pending={null}
      percent={null}
      error={null}
      onCheck={() => undefined}
      onInstall={() => undefined}
      {...overrides}
    />,
  );
}

describe("UpdatePanelView", () => {
  it("浏览器模式只提示不支持，不渲染检查按钮", () => {
    const html = renderPanel({
      desktop: false,
      currentVersion: null,
    });
    expect(html).toContain("浏览器模式不支持应用内更新");
    expect(html).not.toContain("检查更新");
  });

  it("空闲态显示当前版本与检查按钮", () => {
    const html = renderPanel();
    expect(html).toContain("当前版本");
    expect(html).toContain("0.1.1");
    expect(html).toContain("尚未检查更新");
    expect(html).toContain("检查更新");
    expect(html).not.toContain("disabled");
  });

  it("检查中禁用按钮并显示检查中", () => {
    const html = renderPanel({ phase: "checking" });
    expect(html).toContain("检查中…");
    expect(html).toContain("disabled");
  });

  it("已是最新版本不显示安装按钮", () => {
    const html = renderPanel({ phase: "up_to_date" });
    expect(html).toContain("已是最新版本");
    expect(html).not.toContain("下载并安装");
  });

  it("发现新版本时显示版本号、日期、说明与安装按钮", () => {
    const html = renderPanel({ phase: "available", pending });
    expect(html).toContain("发现新版本 v0.1.2");
    expect(html).toContain("2026-11-13T00:00:00Z");
    expect(html).toContain("会话水位");
    expect(html).toContain("下载并安装");
  });

  it("下载中显示确定进度且不重复显示安装按钮", () => {
    const html = renderPanel({
      phase: "downloading",
      pending,
      percent: 42,
    });
    expect(html).toContain('role="progressbar"');
    expect(html).toContain('aria-valuenow="42"');
    expect(html).toContain("width:42%");
    expect(html).not.toContain("下载并安装");
  });

  it("失败态显示清洗后的错误", () => {
    const html = renderPanel({ phase: "failed", error: "网络不可用" });
    expect(html).toContain('role="alert"');
    expect(html).toContain("网络不可用");
  });

  it("重启态提示新版本已下载", () => {
    const html = renderPanel({ phase: "ready", pending });
    expect(html).toContain("更新已下载，正在重启…");
  });

  it("版本未知时显示占位符", () => {
    const html = renderPanel({ currentVersion: null });
    expect(html).toContain("未知");
  });
});

describe("updatePanelReducer", () => {
  it("检查中清空旧结果，检查结果驱动三种阶段", () => {
    const checking = updatePanelReducer(
      {
        ...initialUpdatePanelState,
        phase: "available",
        pending,
        error: "旧错误",
      },
      { type: "check_started" },
    );
    expect(checking.phase).toBe("checking");
    expect(checking.pending).toBeNull();
    expect(checking.error).toBeNull();

    const upToDate = updatePanelReducer(checking, {
      type: "checked",
      outcome: { status: "up_to_date" },
    });
    expect(upToDate.phase).toBe("up_to_date");
    expect(upToDate.pending).toBeNull();

    const available = updatePanelReducer(checking, {
      type: "checked",
      outcome: { status: "available", update: pending },
    });
    expect(available.phase).toBe("available");
    expect(available.pending).toBe(pending);

    const failed = updatePanelReducer(checking, {
      type: "checked",
      outcome: { status: "failed", error: "网络不可用" },
    });
    expect(failed.phase).toBe("failed");
    expect(failed.error).toBe("网络不可用");
  });

  it("安装流程从下载、进度到重启或失败", () => {
    const downloading = updatePanelReducer(
      { ...initialUpdatePanelState, phase: "available", pending },
      { type: "install_started" },
    );
    expect(downloading.phase).toBe("downloading");
    expect(downloading.percent).toBeNull();

    const progress = updatePanelReducer(downloading, {
      type: "progress",
      percent: 42,
    });
    expect(progress.percent).toBe(42);
    expect(updatePanelReducer(progress, { type: "installed" }).phase).toBe(
      "ready",
    );

    const failed = updatePanelReducer(progress, {
      type: "install_failed",
      error: "签名校验失败",
    });
    expect(failed.phase).toBe("failed");
    expect(failed.error).toBe("签名校验失败");
    expect(failed.pending).toBe(pending);
  });

  it("版本加载写入当前版本", () => {
    const state = updatePanelReducer(initialUpdatePanelState, {
      type: "version_loaded",
      version: "0.1.1",
    });
    expect(state.currentVersion).toBe("0.1.1");
  });
});

describe("UpdateBannerView", () => {
  it("无新版本时不渲染", () => {
    const html = renderToStaticMarkup(
      <MemoryRouter>
        <UpdateBannerView version={null} onDismiss={vi.fn()} />
      </MemoryRouter>,
    );
    expect(html).toBe("");
  });

  it("有新版本时显示版本号与设置页入口", () => {
    const html = renderToStaticMarkup(
      <MemoryRouter>
        <UpdateBannerView version="0.1.2" onDismiss={vi.fn()} />
      </MemoryRouter>,
    );
    expect(html).toContain("新版本 v0.1.2 可用");
    expect(html).toContain('href="/settings#settings-update"');
    expect(html).toContain("查看更新");
  });
});
