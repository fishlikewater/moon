import { useCallback, useEffect, useState, type FormEvent } from "react";
import type { RoomPermissionMode } from "@moon/shared";
import { api, errorMessage } from "../api/client";
import { ListSkeleton } from "../components/layout/ListSkeleton";
import { PageHeader } from "../components/layout/PageHeader";
import {
  formatBytes,
  parseSettingsDraft,
  settingsToDraft,
  SETTINGS_LIMITS,
  type SettingsDraft,
  type SettingsDraftErrors,
} from "../settingsView";
import {
  readStoredTheme,
  THEME_CHANGE_EVENT,
  writeThemePreference,
  type ThemePreference,
} from "../useTheme";

const PERMISSION_OPTIONS: ReadonlyArray<{
  value: RoomPermissionMode;
  label: string;
}> = [
  { value: "plan", label: "计划模式" },
  { value: "auto", label: "自主模式" },
  { value: "full", label: "完全访问" },
];

const THEME_OPTIONS: ReadonlyArray<{ value: ThemePreference; label: string }> = [
  { value: "light", label: "亮色" },
  { value: "dark", label: "暗色" },
  { value: "system", label: "跟随系统" },
];

function initialThemePreference(): ThemePreference {
  if (typeof window === "undefined") {
    return "system";
  }
  return readStoredTheme(window.localStorage) ?? "system";
}

/** 设置页：服务端默认值、上下文/附件上限与本地主题偏好。 */
export default function SettingsPage() {
  const [draft, setDraft] = useState<SettingsDraft | null>(null);
  const [errors, setErrors] = useState<SettingsDraftErrors>({});
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  /** 最近一次保存成功的时间戳；驱动 svg 对勾 toast 显示并在 2.5s 后自动消失。 */
  const [savedAt, setSavedAt] = useState<number | null>(null);
  const [themePreference, setThemePreference] =
    useState<ThemePreference>(initialThemePreference);

  const load = useCallback(async () => {
    setLoading(true);
    setLoadError(null);
    try {
      const response = await api.settings.get();
      setDraft(settingsToDraft(response.settings));
    } catch (cause) {
      setLoadError(errorMessage(cause));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  // toast 自动消失；重复保存会写入新时间戳，计时随之重置。
  useEffect(() => {
    if (savedAt === null) {
      return;
    }
    const timer = window.setTimeout(() => setSavedAt(null), 2_500);
    return () => window.clearTimeout(timer);
  }, [savedAt]);

  // 页头主题按钮与设置页按钮共用同一偏好；任一入口切换后同步高亮。
  useEffect(() => {
    if (typeof window === "undefined") {
      return;
    }
    const sync = () => setThemePreference(initialThemePreference());
    window.addEventListener(THEME_CHANGE_EVENT, sync);
    return () => window.removeEventListener(THEME_CHANGE_EVENT, sync);
  }, []);

  const update = <K extends keyof SettingsDraft>(
    key: K,
    value: SettingsDraft[K],
  ) => {
    setDraft((current) => (current === null ? current : { ...current, [key]: value }));
    setSavedAt(null);
    setSaveError(null);
  };

  const chooseTheme = (preference: ThemePreference) => {
    writeThemePreference(preference);
    setThemePreference(preference);
  };

  const save = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (draft === null || saving) {
      return;
    }
    const parsed = parseSettingsDraft(draft);
    if (!parsed.ok) {
      setErrors(parsed.errors);
      setSavedAt(null);
      return;
    }
    setErrors({});
    setSaving(true);
    setSaveError(null);
    try {
      const updated = await api.settings.update(parsed.settings);
      setDraft(settingsToDraft(updated));
      setSavedAt(Date.now());
    } catch (cause) {
      setSaveError(errorMessage(cause));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="page settings-page">
      <PageHeader
        title="设置"
        subtitle="应用级默认值保存在本机数据库；主题与侧栏状态只保存在当前浏览器。"
      />

      {loadError !== null && (
        <div className="settings-banner is-error" role="alert">
          <span>{loadError}</span>
          <button type="button" className="btn btn-secondary btn-sm" onClick={() => void load()}>
            重试
          </button>
        </div>
      )}

      {loading && draft === null ? (
        <ListSkeleton variant="room" count={4} />
      ) : draft === null ? null : (
        <form className="settings-form" onSubmit={save} noValidate>
          {/* 字段错误统一由 parseSettingsDraft 呈现；禁用原生校验避免 min/max/step 气泡拦截提交。 */}
          <section className="card settings-card" aria-labelledby="settings-appearance">
            <h2 className="card-title" id="settings-appearance">
              外观
            </h2>
            <p className="settings-note">主题保存在本机浏览器，不同设备互不影响。</p>
            <div className="settings-segmented" role="group" aria-label="主题">
              {THEME_OPTIONS.map((option) => (
                <button
                  key={option.value}
                  type="button"
                  className="btn btn-secondary"
                  aria-pressed={themePreference === option.value}
                  onClick={() => chooseTheme(option.value)}
                >
                  {option.label}
                </button>
              ))}
            </div>
          </section>

          <section className="card settings-card" aria-labelledby="settings-room">
            <h2 className="card-title" id="settings-room">
              新建房间默认值
            </h2>
            <div className="form-grid">
              <label className="field">
                <span className="field-label">默认权限模式</span>
                <select
                  value={draft.defaultRoomPermissionMode}
                  onChange={(event) =>
                    update(
                      "defaultRoomPermissionMode",
                      event.target.value as RoomPermissionMode,
                    )
                  }
                >
                  {PERMISSION_OPTIONS.map((option) => (
                    <option key={option.value} value={option.value}>
                      {option.label}
                    </option>
                  ))}
                </select>
                <span className="field-hint">只影响新建房间，已有房间不改变。</span>
              </label>
              <label className="field">
                <span className="field-label">默认讨论轮数</span>
                <input
                  type="number"
                  min={SETTINGS_LIMITS.discussionRounds.min}
                  max={SETTINGS_LIMITS.discussionRounds.max}
                  value={draft.defaultRoomDiscussionRounds}
                  onChange={(event) =>
                    update("defaultRoomDiscussionRounds", event.target.value)
                  }
                  aria-invalid={errors.defaultRoomDiscussionRounds !== undefined}
                />
                {errors.defaultRoomDiscussionRounds !== undefined ? (
                  <span className="field-error">{errors.defaultRoomDiscussionRounds}</span>
                ) : (
                  <span className="field-hint">
                    {SETTINGS_LIMITS.discussionRounds.min}–
                    {SETTINGS_LIMITS.discussionRounds.max} 轮。
                  </span>
                )}
              </label>
            </div>
          </section>

          <section className="card settings-card" aria-labelledby="settings-context">
            <h2 className="card-title" id="settings-context">
              上下文
            </h2>
            <label className="settings-check">
              <input
                type="checkbox"
                checked={draft.contextCompressionEnabled}
                onChange={(event) =>
                  update("contextCompressionEnabled", event.target.checked)
                }
              />
              <span>长对话自动压缩：保留最近消息全文，较早内容压成摘要。</span>
            </label>
            <div className="form-grid">
              <label className="field">
                <span className="field-label">上下文字符预算</span>
                <input
                  type="number"
                  min={SETTINGS_LIMITS.contextBudgetChars.min}
                  max={SETTINGS_LIMITS.contextBudgetChars.max}
                  step={1000}
                  value={draft.contextBudgetChars}
                  onChange={(event) => update("contextBudgetChars", event.target.value)}
                  aria-invalid={errors.contextBudgetChars !== undefined}
                  disabled={!draft.contextCompressionEnabled}
                />
                {errors.contextBudgetChars !== undefined ? (
                  <span className="field-error">{errors.contextBudgetChars}</span>
                ) : (
                  <span className="field-hint">
                    {SETTINGS_LIMITS.contextBudgetChars.min.toLocaleString()}–
                    {SETTINGS_LIMITS.contextBudgetChars.max.toLocaleString()} 字符；约{" "}
                    {Math.ceil(Number(draft.contextBudgetChars || 0) / 4).toLocaleString()}{" "}
                    tokens（按 4 字符约 1 token 估算，仅供展示）。
                  </span>
                )}
              </label>
            </div>
          </section>

          <section className="card settings-card" aria-labelledby="settings-attachments">
            <h2 className="card-title" id="settings-attachments">
              附件
            </h2>
            <div className="form-grid">
              <label className="field">
                <span className="field-label">单个附件上限（字节）</span>
                <input
                  type="number"
                  min={SETTINGS_LIMITS.attachmentMaxFileBytes.min}
                  max={SETTINGS_LIMITS.attachmentMaxFileBytes.max}
                  step={1_048_576}
                  value={draft.attachmentMaxFileBytes}
                  onChange={(event) =>
                    update("attachmentMaxFileBytes", event.target.value)
                  }
                  aria-invalid={errors.attachmentMaxFileBytes !== undefined}
                />
                {errors.attachmentMaxFileBytes !== undefined ? (
                  <span className="field-error">{errors.attachmentMaxFileBytes}</span>
                ) : (
                  <span className="field-hint">
                    当前约 {formatBytes(Number(draft.attachmentMaxFileBytes || 0))}。
                  </span>
                )}
              </label>
              <label className="field">
                <span className="field-label">单条消息附件数</span>
                <input
                  type="number"
                  min={SETTINGS_LIMITS.attachmentMaxFilesPerMessage.min}
                  max={SETTINGS_LIMITS.attachmentMaxFilesPerMessage.max}
                  value={draft.attachmentMaxFilesPerMessage}
                  onChange={(event) =>
                    update("attachmentMaxFilesPerMessage", event.target.value)
                  }
                  aria-invalid={
                    errors.attachmentMaxFilesPerMessage !== undefined
                  }
                />
                {errors.attachmentMaxFilesPerMessage !== undefined ? (
                  <span className="field-error">
                    {errors.attachmentMaxFilesPerMessage}
                  </span>
                ) : (
                  <span className="field-hint">
                    {SETTINGS_LIMITS.attachmentMaxFilesPerMessage.min}–
                    {SETTINGS_LIMITS.attachmentMaxFilesPerMessage.max} 个。
                  </span>
                )}
              </label>
            </div>
          </section>

          <section className="card settings-card" aria-labelledby="settings-runtime">
            <h2 className="card-title" id="settings-runtime">
              运行信息
            </h2>
            <div className="form-grid">
              <label className="field">
                <span className="field-label">运行总时长上限（秒）</span>
                <input
                  type="number"
                  min={0}
                  max={SETTINGS_LIMITS.runTimeoutSecs.max}
                  step={60}
                  value={draft.runTimeoutSecs}
                  onChange={(event) => update("runTimeoutSecs", event.target.value)}
                  aria-invalid={errors.runTimeoutSecs !== undefined}
                />
                {errors.runTimeoutSecs !== undefined ? (
                  <span className="field-error">{errors.runTimeoutSecs}</span>
                ) : (
                  <span className="field-hint">
                    {SETTINGS_LIMITS.runTimeoutSecs.min}–
                    {SETTINGS_LIMITS.runTimeoutSecs.max} 秒；0 表示不限制。单次
                    harness 运行超过该时长会被终止。
                  </span>
                )}
              </label>
              <div className="field">
                <span className="field-label">数据保存</span>
                <span className="field-hint">
                  本机数据库（不含云端同步）
                </span>
              </div>
            </div>
          </section>

          {saveError !== null && (
            <div className="settings-banner is-error" role="alert">
              {saveError}
            </div>
          )}
          {savedAt !== null && (
            <div className="settings-toast" role="status">
              <span className="settings-toast-icon" aria-hidden="true">
                <svg viewBox="0 0 16 16" width="10" height="10" aria-hidden="true">
                  <path
                    d="M3 8.5l3.2 3.2L13 5"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="2"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  />
                </svg>
              </span>
              设置已保存
            </div>
          )}

          <div className="form-actions">
            <button type="submit" className="btn btn-primary" disabled={saving}>
              {saving ? "保存中…" : "保存设置"}
            </button>
            <button
              type="button"
              className="btn btn-secondary"
              disabled={saving}
              onClick={() => void load()}
            >
              重新加载
            </button>
          </div>
        </form>
      )}
    </div>
  );
}
