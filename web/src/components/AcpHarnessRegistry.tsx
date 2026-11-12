import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { CustomHarness } from "@moon/shared";
import { api, errorMessage } from "../api/client";
import { AcpCapabilityDetail } from "./acpCapabilityDetail";
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

  const probe = async (item: CustomHarness) => {
    setProbeError(null);
    setProbingId(item.id);
    try {
      await api.customHarnesses.probe(item.id);
      // 重新读取列表：服务端已写入 capabilitiesJson/probedAt，页面也需要最新快照。
      await reload();
    } catch (cause) {
      setProbeError({ id: item.id, message: errorMessage(cause) });
    } finally {
      setProbingId(null);
    }
  };

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
            probeError?.id === capabilityItem.id ? probeError.message : null
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
                        <span className="badge">
                          {inUse ? `引用 ${item.agentCount}` : "未引用"}
                        </span>
                        {envCount > 0 && (
                          <span className="badge">环境变量 {envCount}</span>
                        )}
                      </div>
                      <code className="acp-harness-command">{commandLine}</code>
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
                        aria-label={`探测自定义 ACP agent ${item.name}`}
                        onClick={() => void probe(item)}
                        disabled={busy}
                      >
                        {probingId === item.id ? "探测中…" : "探测模型/权限"}
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
