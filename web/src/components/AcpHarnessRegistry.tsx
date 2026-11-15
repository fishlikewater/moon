import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { CustomHarness } from "@moon/shared";
import { api, errorMessage } from "../api/client";
import { AcpCapabilityDetail, formatProbedAt } from "./acpCapabilityDetail";
import { AcpHarnessFormModal } from "./AcpHarnessFormModal";
import {
  indexRegistryByPackage,
  resolveEntryUpdateState,
  type EntryUpdateState,
} from "../acpRegistryCatalog";
import { useAcpRegistry } from "../useAcpRegistry";

interface AcpHarnessRegistryProps {
  /** 注册表变化后回传最新列表，供页面统计条目数；可选。 */
  onChanged?: (items: CustomHarness[]) => void;
}

/** 首次探测放宽上限：npx 首次运行需要下载依赖，默认 15 秒会中途失败。 */
export const FIRST_PROBE_TIMEOUT_SECS = 90;

/** 条目可用性三态：探测成功 / 失败（含原因）/ 从未探测。 */
export type AcpAvailability = {
  tone: "ready" | "error" | "pending";
  label: string;
};

/** 纯函数：从条目快照推导可用性三态，供徽标与测试共用。 */
export function resolveAcpAvailability(
  item: Pick<CustomHarness, "capabilities" | "probeError">,
): AcpAvailability {
  // 最近一次失败优先展示：曾成功后失败（如卸载 Node）必须显示不可用，
  // 否则「装/卸 Node 的可解释性」这个核心场景永远不可达。
  if (item.probeError !== null) {
    return { tone: "error", label: "不可用" };
  }
  if (item.capabilities !== null) {
    return {
      tone: "ready",
      label: `已探测 · ${item.capabilities.models.length} 模型 ${item.capabilities.modes.length} 模式`,
    };
  }
  return { tone: "pending", label: "未检测" };
}

/** 纯函数：是否应该自动探测（从未探测过，无论成功失败都不重复 spawn）。 */
export function shouldAutoProbe(
  item: Pick<CustomHarness, "capabilities" | "probeError">,
): boolean {
  return item.capabilities === null && item.probeError === null;
}

/** 纯函数：按列表顺序取下一个应自动探测的条目；没有则返回 null。 */
export function nextAutoProbeTarget(
  items: readonly Pick<
    CustomHarness,
    "id" | "capabilities" | "probeError"
  >[],
  probedIds: ReadonlySet<number>,
): Pick<CustomHarness, "id" | "capabilities" | "probeError"> | null {
  return (
    items.find(
      (item) => shouldAutoProbe(item) && !probedIds.has(item.id),
    ) ?? null
  );
}

/** 纯函数：探测超时——从未探测放宽到 90 秒，其余走服务端默认（15 秒）。 */
export function probeTimeoutFor(
  item: Pick<CustomHarness, "capabilities" | "probeError">,
): number | undefined {
  return shouldAutoProbe(item) ? FIRST_PROBE_TIMEOUT_SECS : undefined;
}

/** 纯函数：任一条目当前可用（最近一次失败优先于旧成功快照，与三态徽标一致）。 */
export function acpUsableFromItems(
  items: readonly Pick<CustomHarness, "capabilities" | "probeError">[],
): boolean {
  return items.some((item) => resolveAcpAvailability(item).tone === "ready");
}

/** 纯函数：相对时间（刚刚 / N 分钟前 / N 小时前 / 超过一天回退绝对时间）。 */
export function formatRelativeTime(value: number, now: number = Date.now()): string {
  const deltaSeconds = Math.max(0, Math.round((now - value) / 1000));
  if (deltaSeconds < 60) {
    return "刚刚";
  }
  if (deltaSeconds < 3600) {
    return `${Math.round(deltaSeconds / 60)} 分钟前`;
  }
  if (deltaSeconds < 86400) {
    return `${Math.round(deltaSeconds / 3600)} 小时前`;
  }
  return formatProbedAt(value);
}

/**
 * 自定义 ACP agent 注册表：列表、新建/编辑、能力详情与删除。
 *
 * 从原智能体页模态框迁出，作为 Harness 页面的内联区块；列表保持内联，
 * 新建/编辑表单在弹层中打开。registry 目录驱动预设选择器与版本更新提示。
 */
export function AcpHarnessRegistry({ onChanged }: AcpHarnessRegistryProps) {
  const [items, setItems] = useState<CustomHarness[]>([]);
  const [loading, setLoading] = useState(true);
  const [listError, setListError] = useState<string | null>(null);
  const [deletingId, setDeletingId] = useState<number | null>(null);
  const [probingId, setProbingId] = useState<number | null>(null);
  const [probeError, setProbeError] = useState<{
    id: number;
    message: string;
  } | null>(null);
  const [updatingId, setUpdatingId] = useState<number | null>(null);
  const [updateError, setUpdateError] = useState<{
    id: number;
    message: string;
  } | null>(null);
  const [formTarget, setFormTarget] = useState<
    CustomHarness | "create" | null
  >(null);
  /** 非 null 时显示该条目的能力详情视图；与表单弹层互斥。 */
  const [capabilityViewId, setCapabilityViewId] = useState<number | null>(
    null,
  );
  const containerRef = useRef<HTMLElement | null>(null);
  const registry = useAcpRegistry();

  const registryByPackage = useMemo(
    () =>
      indexRegistryByPackage(
        registry.status === "ready" ? registry.payload : null,
      ),
    [registry],
  );

  // 详情视图按 id 实时查找条目：探测/删除刷新列表后内容与视图状态同步。
  const capabilityItem =
    capabilityViewId === null
      ? null
      : (items.find((item) => item.id === capabilityViewId) ?? null);

  const reload = useCallback(async (options?: { showLoading?: boolean }) => {
    if (options?.showLoading !== false) {
      setLoading(true);
    }
    setListError(null);
    try {
      const next = await api.customHarnesses.list();
      setItems(next);
      onChanged?.(next);
    } catch (cause) {
      setListError(errorMessage(cause));
    } finally {
      setLoading(false);
    }
  }, [onChanged]);

  useEffect(() => {
    void reload();
  }, [reload]);

  const startCreate = () => {
    setCapabilityViewId(null);
    setFormTarget("create");
  };

  const startEdit = (item: CustomHarness) => {
    setCapabilityViewId(null);
    setFormTarget(item);
  };

  const openCapability = (item: CustomHarness) => {
    setFormTarget(null);
    setCapabilityViewId(item.id);
  };

  const closeCapability = () => {
    setCapabilityViewId(null);
    containerRef.current?.focus();
  };

  const closeForm = useCallback(() => {
    setFormTarget(null);
  }, []);

  const handleFormSaved = useCallback(() => {
    setFormTarget(null);
    // 保存后静默刷新：列表保持挂载，弹层卸载时触发按钮仍在，焦点才能归还。
    void reload({ showLoading: false });
  }, [reload]);

  const probe = useCallback(
    async (item: Pick<CustomHarness, "id">, timeoutSecs?: number) => {
      setProbeError(null);
      setProbingId(item.id);
      try {
        await api.customHarnesses.probe(item.id, timeoutSecs);
        // 服务端已写入 capabilities/probedAt，页面需要最新快照。
        await reload();
      } catch (cause) {
        setProbeError({ id: item.id, message: errorMessage(cause) });
        // 失败原因已落库：刷新列表让持久徽标立即显示，不重复弹错误。
        await reload({ showLoading: false }).catch(() => undefined);
      } finally {
        setProbingId(null);
      }
    },
    [reload],
  );

  // 新手引导的自动探测：从未探测的条目在列表就绪后串行探测一次。
  //
  // 并发恒为 1：autoProbeRunningRef 保证任一时刻只有一个循环在飞（失败
  // reload 会让 items 变化并重启 effect，但运行中的循环不受影响，继续从
  // itemsRef 取最新列表）；每轮目标从最新 itemsRef + 已处理集合计算，
  // 失败条目也会标记，避免重复 spawn。
  const itemsRef = useRef<CustomHarness[]>([]);
  const autoProbedRef = useRef<Set<number>>(new Set());
  const autoProbeRunningRef = useRef(false);
  useEffect(() => {
    itemsRef.current = items;
  }, [items]);
  useEffect(() => {
    if (loading || autoProbeRunningRef.current) {
      return;
    }
    if (nextAutoProbeTarget(items, autoProbedRef.current) === null) {
      return;
    }
    autoProbeRunningRef.current = true;
    void (async () => {
      try {
        for (;;) {
          const target = nextAutoProbeTarget(itemsRef.current, autoProbedRef.current);
          if (target === null) {
            return;
          }
          autoProbedRef.current.add(target.id);
          await probe(target, FIRST_PROBE_TIMEOUT_SECS);
        }
      } finally {
        autoProbeRunningRef.current = false;
      }
    })();
  }, [loading, items, probe]);

  const remove = async (item: CustomHarness) => {
    if (!window.confirm(`确认删除自定义 ACP agent「${item.name}」？`)) {
      return;
    }
    setListError(null);
    setDeletingId(item.id);
    try {
      await api.customHarnesses.remove(item.id);
      await reload();
      containerRef.current?.focus();
    } catch (cause) {
      setListError(errorMessage(cause));
    } finally {
      setDeletingId(null);
    }
  };

  // 一键更新：把条目参数中的包 token 改写为 registry 最新版本。
  const updateToLatest = async (item: CustomHarness, args: string[]) => {
    if (!window.confirm(`把「${item.name}」的包参数更新到最新版本？`)) {
      return;
    }
    setUpdateError(null);
    setUpdatingId(item.id);
    try {
      await api.customHarnesses.update(item.id, { args });
      await reload({ showLoading: false });
    } catch (cause) {
      setUpdateError({ id: item.id, message: errorMessage(cause) });
    } finally {
      setUpdatingId(null);
    }
  };

  return (
    <section
      className="card harness-registry"
      id="acp-registry"
      aria-labelledby="acp-registry-title"
      ref={containerRef}
      tabIndex={-1}
    >
      <header className="harness-section-head">
        <h2 className="card-title" id="acp-registry-title">
          自定义 ACP agent
        </h2>
        {capabilityItem === null && (
          <button
            type="button"
            className="btn btn-primary btn-sm"
            onClick={startCreate}
          >
            新建 ACP agent
          </button>
        )}
      </header>

      <p className="field-hint">
        自定义 ACP agent 是本地可执行的 ACP 程序；参数每行一个，环境变量每行
        KEY=VALUE。
      </p>

      {capabilityItem !== null ? (
        <AcpCapabilityDetail
          name={capabilityItem.name}
          capability={capabilityItem.capabilities}
          probedAt={capabilityItem.probedAt}
          probeError={
            // 即时失败优先；否则展示持久化的最近失败，避免详情页只报喜不报忧。
            probeError?.id === capabilityItem.id
              ? probeError.message
              : capabilityItem.probeError
          }
          onBack={closeCapability}
        />
      ) : (
        <>
          {listError !== null && (
            <p className="banner error" role="alert">
              {listError}
            </p>
          )}

          {loading ? (
            <p className="field-hint" role="status">
              正在读取注册表…
            </p>
          ) : items.length === 0 ? (
            <p className="field-hint">
              注册表为空：点击「新建 ACP agent」注册一个本地 ACP 程序。
            </p>
          ) : (
            <ul className="acp-harness-list" aria-label="自定义 ACP agent 列表">
              {items.map((item) => {
                const envCount = Object.keys(item.env).length;
                const commandLine = [item.command, ...item.args].join(" ");
                const inUse = item.agentCount > 0;
                const availability = resolveAcpAvailability(item);
                const availabilityClass =
                  availability.tone === "ready"
                    ? "badge-ready"
                    : availability.tone === "error"
                      ? "badge-error"
                      : "badge-progress";
                const busy =
                  probingId !== null ||
                  deletingId !== null ||
                  updatingId !== null;
                const updateState: EntryUpdateState = resolveEntryUpdateState(
                  item,
                  registryByPackage,
                );
                return (
                  <li key={item.id} className="acp-harness-item">
                    <div className="acp-harness-body">
                      <div className="acp-harness-title-row">
                        <h3 className="acp-harness-name">{item.name}</h3>
                        <span className={`badge ${availabilityClass}`}>
                          {availability.label}
                        </span>
                        <span className="badge">
                          {inUse ? `引用 ${item.agentCount}` : "未引用"}
                        </span>
                        {envCount > 0 && (
                          <span className="badge">环境变量 {envCount}</span>
                        )}
                      </div>
                      <code className="acp-harness-command">{commandLine}</code>
                      {item.probeError !== null && (
                        <p className="field-hint" role="alert">
                          {item.probeError}
                          {item.probeFailedAt !== null &&
                            `（检测于 ${formatRelativeTime(item.probeFailedAt)}）`}
                        </p>
                      )}
                      {inUse && (
                        <p className="field-hint">
                          正被 {item.agentCount} 个智能体引用，先改绑这些智能体后才能删除。
                        </p>
                      )}
                      {updateState.kind === "update-available" && (
                        <div className="acp-harness-update-row">
                          <span className="badge">
                            可更新 {updateState.currentVersion} →{" "}
                            {updateState.latestVersion}
                          </span>
                          <button
                            type="button"
                            className="btn btn-secondary btn-sm"
                            aria-label={`把 ${item.name} 更新到 ${updateState.latestVersion}`}
                            onClick={() =>
                              void updateToLatest(item, updateState.updatedArgs)
                            }
                            disabled={busy}
                          >
                            {updatingId === item.id
                              ? "更新中…"
                              : `更新到 ${updateState.latestVersion}`}
                          </button>
                          {updateError?.id === item.id && (
                            <span className="field-hint" role="alert">
                              {updateError.message}
                            </span>
                          )}
                        </div>
                      )}
                      {updateState.kind === "unpinned" && (
                        <p className="field-hint">
                          未锁版本；registry 最新 {updateState.latestVersion}。
                        </p>
                      )}
                      {updateState.kind === "incomparable" && (
                        <p className="field-hint">
                          当前 {updateState.currentVersion}；registry 最新{" "}
                          {updateState.latestVersion}。
                        </p>
                      )}
                    </div>

                    <div className="agent-actions">
                      <button
                        type="button"
                        className="btn btn-secondary btn-sm"
                        aria-label={`检测自定义 ACP agent ${item.name} 的能力`}
                        onClick={() =>
                          void probe(item, probeTimeoutFor(item))
                        }
                        disabled={busy}
                      >
                        {probingId === item.id ? "检测中…" : "检测能力"}
                      </button>
                      <button
                        type="button"
                        className="btn btn-secondary btn-sm"
                        aria-label={`编辑自定义 ACP agent ${item.name}`}
                        onClick={() => startEdit(item)}
                        disabled={busy}
                      >
                        编辑
                      </button>
                      <button
                        type="button"
                        className="btn btn-secondary btn-sm"
                        aria-label={`查看自定义 ACP agent ${item.name} 的探测能力`}
                        onClick={() => openCapability(item)}
                      >
                        能力
                      </button>
                      <button
                        type="button"
                        className="btn btn-danger btn-sm"
                        aria-label={`删除自定义 ACP agent ${item.name}`}
                        onClick={() => void remove(item)}
                        disabled={inUse || busy}
                        title={
                          inUse
                            ? `正被 ${item.agentCount} 个智能体引用，无法删除`
                            : undefined
                        }
                      >
                        {deletingId === item.id ? "删除中…" : "删除"}
                      </button>
                    </div>
                  </li>
                );
              })}
            </ul>
          )}
        </>
      )}

      {formTarget !== null && (
        <AcpHarnessFormModal
          item={formTarget === "create" ? null : formTarget}
          registry={registry}
          onClose={closeForm}
          onSaved={handleFormSaved}
        />
      )}
    </section>
  );
}
