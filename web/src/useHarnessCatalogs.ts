/**
 * 内置 harness 模型目录的读取状态。
 *
 * 目录来自只读接口 `GET /api/harnesses/:harness/models`；单个 harness 读取
 * 失败只影响自己的卡片，不阻塞页面其他内容。
 */
import { useEffect, useState } from "react";
import type { HarnessKind, HarnessModelCatalog } from "@moon/shared";
import { api, errorMessage } from "./api/client";
import type { HarnessCatalogState } from "./harnessCatalog";

export type HarnessCatalogLoader = (
  kind: HarnessKind,
) => Promise<HarnessModelCatalog>;

function loadingStates(
  kinds: readonly HarnessKind[],
): Record<string, HarnessCatalogState> {
  const states: Record<string, HarnessCatalogState> = {};
  for (const kind of kinds) {
    states[kind] = { status: "loading" };
  }
  return states;
}

/**
 * 并发读取多个 harness 的模型目录。
 *
 * 用 `allSettled` 隔离失败：被拒绝的条目映射为 error，其余条目照常 ready。
 * `load` 可注入，测试无需真实网络。
 */
export async function loadHarnessCatalogs(
  kinds: readonly HarnessKind[],
  load: HarnessCatalogLoader = api.harnesses.models,
): Promise<Record<string, HarnessCatalogState>> {
  const results = await Promise.allSettled(
    kinds.map((kind) => Promise.resolve().then(() => load(kind))),
  );

  const states: Record<string, HarnessCatalogState> = {};
  kinds.forEach((kind, index) => {
    const result = results[index];
    if (result === undefined) {
      states[kind] = { status: "error", message: "模型目录请求缺失" };
      return;
    }
    states[kind] =
      result.status === "fulfilled"
        ? { status: "ready", catalog: result.value }
        : { status: "error", message: errorMessage(result.reason) };
  });
  return states;
}

/**
 * 页面级目录状态 hook；组件卸载后丢弃迟到结果。
 *
 * `kinds` 的内容身份由拼接键表示，调用方传稳定常量即可避免重复请求。
 */
export function useHarnessCatalogs(
  kinds: readonly HarnessKind[],
  load: HarnessCatalogLoader = api.harnesses.models,
): Record<string, HarnessCatalogState> {
  const [states, setStates] = useState<Record<string, HarnessCatalogState>>(
    () => loadingStates(kinds),
  );
  const kindsKey = kinds.join(",");

  useEffect(() => {
    let cancelled = false;
    setStates(loadingStates(kinds));
    void loadHarnessCatalogs(kinds, load).then((next) => {
      if (!cancelled) {
        setStates(next);
      }
    });
    return () => {
      cancelled = true;
    };
    // kindsKey 代表 kinds 的内容；调用方使用稳定数组时无需重复读取。
  }, [kindsKey, load]);

  return states;
}
