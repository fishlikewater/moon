import type { HarnessKind, HarnessModelCatalog } from "@moon/shared";
import { describe, expect, it } from "vitest";
import { loadHarnessCatalogs } from "./useHarnessCatalogs";

function catalog(harness: HarnessKind): HarnessModelCatalog {
  return { harness, available: true, models: [], message: null };
}

describe("loadHarnessCatalogs", () => {
  it("单个 harness 失败只映射该条目，其余照常可用", async () => {
    const states = await loadHarnessCatalogs(
      ["zcode", "codex"],
      async (kind) => {
        if (kind === "codex") {
          throw new Error("offline");
        }
        return catalog(kind);
      },
    );

    expect(states.zcode).toEqual({
      status: "ready",
      catalog: catalog("zcode"),
    });
    expect(states.codex).toEqual({ status: "error", message: "offline" });
  });

  it("同步抛出的 loader 也被隔离", async () => {
    const states = await loadHarnessCatalogs(["kimi-code"], () => {
      throw new Error("sync failure");
    });

    expect(states["kimi-code"]).toEqual({
      status: "error",
      message: "sync failure",
    });
  });

  it("空目录保留 ready 状态，由文案层解释不可用原因", async () => {
    const unavailable: HarnessModelCatalog = {
      harness: "kimi-code",
      available: false,
      models: [],
      message: "未找到 Kimi Code 配置",
    };

    const states = await loadHarnessCatalogs(
      ["kimi-code"],
      async () => unavailable,
    );

    expect(states["kimi-code"]).toEqual({
      status: "ready",
      catalog: unavailable,
    });
  });
});
