import { describe, expect, it } from "vitest";
import { loadAcpRegistry } from "./useAcpRegistry";
import type { AcpRegistryPayload } from "./acpRegistryCatalog";

function payload(): AcpRegistryPayload {
  return {
    agents: [
      {
        id: "demo",
        name: "Demo",
        version: "1.0.0",
        summary: "",
        npx: { package: "demo@1.0.0", args: [] },
        windowsBinary: null,
      },
    ],
    source: "network",
    fetchedAt: 1,
  };
}

describe("loadAcpRegistry", () => {
  it("成功拉取映射为 ready", async () => {
    const state = await loadAcpRegistry(async () => payload());
    expect(state).toEqual({ status: "ready", payload: payload() });
  });

  it("失败映射为 error 且保留错误信息", async () => {
    const state = await loadAcpRegistry(async () => {
      throw new Error("registry offline");
    });
    expect(state).toEqual({ status: "error", message: "registry offline" });
  });

  it("同步抛出的 loader 也被隔离", async () => {
    const state = await loadAcpRegistry(() => {
      throw new Error("sync failure");
    });
    expect(state).toEqual({ status: "error", message: "sync failure" });
  });
});
