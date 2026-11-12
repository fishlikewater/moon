/**
 * ACP registry 目录的读取状态。
 *
 * 目录来自只读接口 `GET /api/acp-registry`（后端代理：CDN + TTL 缓存 + 快照
 * 回退）；读取失败时返回 error，由调用方决定回退行为（如旧模板）。
 */
import { useEffect, useRef, useState } from "react";
import { api, errorMessage } from "./api/client";
import type { AcpRegistryPayload } from "./acpRegistryCatalog";

export type AcpRegistryLoader = () => Promise<AcpRegistryPayload>;

export type AcpRegistryState =
  | { status: "loading" }
  | { status: "ready"; payload: AcpRegistryPayload }
  | { status: "error"; message: string };

/**
 * 默认 loader：模块级稳定引用。
 *
 * 不能写成 `() => api.acpRegistry.list()` 这类内联箭头——默认参数每次渲染
 * 都会生成新函数身份，曾导致 effect 每轮重跑、页面无限重复请求（复核发现的
 * blocking 缺陷）。
 */
const DEFAULT_REGISTRY_LOADER: AcpRegistryLoader = api.acpRegistry.list;

/** 单次读取：成功 → ready，失败（含同步抛出）→ error。 */
export async function loadAcpRegistry(
  load: AcpRegistryLoader = DEFAULT_REGISTRY_LOADER,
): Promise<AcpRegistryState> {
  try {
    const payload = await Promise.resolve().then(() => load());
    return { status: "ready", payload };
  } catch (cause) {
    return { status: "error", message: errorMessage(cause) };
  }
}

/**
 * 页面级 registry 状态 hook；组件卸载后丢弃迟到结果。
 *
 * 挂载后只读取一次：loader 经 ref 间接调用，调用方传内联 loader 也不会
 * 触发重复请求（本仓库无 jsdom，hook 无法挂载测试，稳定性由结构保证）。
 */
export function useAcpRegistry(
  load: AcpRegistryLoader = DEFAULT_REGISTRY_LOADER,
): AcpRegistryState {
  const [state, setState] = useState<AcpRegistryState>({ status: "loading" });
  const loadRef = useRef(load);

  useEffect(() => {
    loadRef.current = load;
  }, [load]);

  useEffect(() => {
    let cancelled = false;
    void loadAcpRegistry((...args) => loadRef.current(...args)).then((next) => {
      if (!cancelled) {
        setState(next);
      }
    });
    return () => {
      cancelled = true;
    };
    // 挂载时读取一次；loader 变化经 loadRef 生效，不触发重新拉取。
  }, []);

  return state;
}
