/**
 * 设置页「软件更新」区块。
 *
 * 容器只编排状态；`UpdatePanelView` 是纯视图，便于按阶段静态断言文案与按钮。
 */

import { useCallback, useEffect, useReducer } from "react";
import { isDesktopShell } from "../desktopShell";
import {
  getCurrentVersion,
  runUpdateCheck,
  runUpdateInstall,
  updateStatusText,
  type CheckOutcome,
  type PendingUpdate,
  type UpdatePhase,
} from "../updater";

export type UpdatePanelViewProps = {
  desktop: boolean;
  phase: UpdatePhase;
  currentVersion: string | null;
  pending: PendingUpdate | null;
  percent: number | null;
  error: string | null;
  onCheck: () => void;
  onInstall: () => void;
};

export function UpdatePanelView({
  desktop,
  phase,
  currentVersion,
  pending,
  percent,
  error,
  onCheck,
  onInstall,
}: UpdatePanelViewProps) {
  const busy = phase === "checking" || phase === "downloading";
  return (
    <section
      className="card settings-card"
      id="settings-update"
      aria-labelledby="settings-update-title"
    >
      <h2 className="card-title" id="settings-update-title">
        软件更新
      </h2>
      {!desktop ? (
        <p className="settings-note">
          浏览器模式不支持应用内更新；请使用桌面端检查新版本。
        </p>
      ) : (
        <>
          <p className="settings-note">
            当前版本：
            <span className="update-version">{currentVersion ?? "未知"}</span>
          </p>
          <p className="settings-note" role="status">
            {updateStatusText(phase, pending?.info.version)}
          </p>

          {phase === "downloading" && (
            <div className="update-progress-block">
              {percent === null ? (
                <p className="settings-note">正在下载，进度未知…</p>
              ) : (
                <div
                  className="update-progress"
                  role="progressbar"
                  aria-label="更新下载进度"
                  aria-valuemin={0}
                  aria-valuemax={100}
                  aria-valuenow={percent}
                >
                  <span
                    className="update-progress-bar"
                    style={{ width: `${percent}%` }}
                  />
                </div>
              )}
            </div>
          )}

          {pending !== null &&
            phase !== "downloading" &&
            phase !== "ready" && (
              <div className="update-notes">
                {pending.info.date !== null && (
                  <p className="settings-note">
                    发布日期：{pending.info.date}
                  </p>
                )}
                {pending.info.body !== null && (
                  <pre className="update-notes-body">{pending.info.body}</pre>
                )}
              </div>
            )}

          {error !== null && (
            <p className="field-error" role="alert">
              {error}
            </p>
          )}

          <div className="form-actions">
            <button
              type="button"
              className="btn btn-secondary"
              disabled={busy}
              onClick={onCheck}
            >
              {phase === "checking" ? "检查中…" : "检查更新"}
            </button>
            {phase === "available" && pending !== null && (
              <button
                type="button"
                className="btn btn-primary"
                onClick={onInstall}
              >
                下载并安装
              </button>
            )}
          </div>
        </>
      )}
    </section>
  );
}

/** 容器：读取当前版本，执行检查与安装状态流转。 */
export type UpdatePanelState = {
  phase: UpdatePhase;
  currentVersion: string | null;
  pending: PendingUpdate | null;
  percent: number | null;
  error: string | null;
};

export const initialUpdatePanelState: UpdatePanelState = {
  phase: "idle",
  currentVersion: null,
  pending: null,
  percent: null,
  error: null,
};

export type UpdatePanelEvent =
  | { type: "version_loaded"; version: string | null }
  | { type: "check_started" }
  | { type: "checked"; outcome: CheckOutcome }
  | { type: "install_started" }
  | { type: "progress"; percent: number | null }
  | { type: "installed" }
  | { type: "install_failed"; error: string };

/** 状态编排是纯函数，容器只负责调用 updater 并派发事件。 */
export function updatePanelReducer(
  state: UpdatePanelState,
  event: UpdatePanelEvent,
): UpdatePanelState {
  switch (event.type) {
    case "version_loaded":
      return { ...state, currentVersion: event.version };
    case "check_started":
      return {
        ...state,
        phase: "checking",
        pending: null,
        percent: null,
        error: null,
      };
    case "checked":
      if (event.outcome.status === "up_to_date") {
        return { ...state, phase: "up_to_date", pending: null };
      }
      if (event.outcome.status === "failed") {
        return { ...state, phase: "failed", error: event.outcome.error };
      }
      return { ...state, phase: "available", pending: event.outcome.update };
    case "install_started":
      return { ...state, phase: "downloading", percent: null, error: null };
    case "progress":
      return { ...state, percent: event.percent };
    case "installed":
      return { ...state, phase: "ready" };
    case "install_failed":
      return { ...state, phase: "failed", error: event.error };
  }
}

export default function UpdatePanel() {
  const desktop = isDesktopShell();
  const [state, dispatch] = useReducer(
    updatePanelReducer,
    initialUpdatePanelState,
  );

  useEffect(() => {
    if (!desktop) {
      return;
    }
    let active = true;
    void getCurrentVersion().then((version) => {
      if (active) {
        dispatch({ type: "version_loaded", version });
      }
    });
    return () => {
      active = false;
    };
  }, [desktop]);

  const check = useCallback(async () => {
    if (!desktop) {
      return;
    }
    dispatch({ type: "check_started" });
    dispatch({ type: "checked", outcome: await runUpdateCheck() });
  }, [desktop]);

  const install = useCallback(async () => {
    if (state.pending === null) {
      return;
    }
    dispatch({ type: "install_started" });
    const outcome = await runUpdateInstall(state.pending, (progress) => {
      dispatch({ type: "progress", percent: progress.percent });
    });
    if (!outcome.ok) {
      dispatch({ type: "install_failed", error: outcome.error });
      return;
    }
    dispatch({ type: "installed" });
  }, [state.pending]);

  return (
    <UpdatePanelView
      desktop={desktop}
      phase={state.phase}
      currentVersion={state.currentVersion}
      pending={state.pending}
      percent={state.percent}
      error={state.error}
      onCheck={() => void check()}
      onInstall={() => void install()}
    />
  );
}
