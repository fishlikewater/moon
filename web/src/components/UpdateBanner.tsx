/**
 * 桌面启动后的非阻断更新提示：只提示、不自动下载。
 *
 * 检查失败或非桌面壳时静默；是否展示由视图的 version 决定。
 */

import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import { isDesktopShell } from "../desktopShell";
import { runUpdateCheck } from "../updater";

export function UpdateBannerView({
  version,
  onDismiss,
}: {
  version: string | null;
  onDismiss: () => void;
}) {
  if (version === null) {
    return null;
  }
  return (
    <div className="update-banner" role="status">
      <span className="update-banner-text">新版本 v{version} 可用</span>
      <Link className="update-banner-link" to="/settings#settings-update">
        查看更新
      </Link>
      <button
        type="button"
        className="update-banner-close"
        aria-label="关闭更新提示"
        onClick={onDismiss}
      >
        ×
      </button>
    </div>
  );
}

/** 启动后延迟检查一次；发现新版本时展示提示，不自动下载。 */
export default function UpdateBanner() {
  const desktop = isDesktopShell();
  const [version, setVersion] = useState<string | null>(null);
  const [dismissed, setDismissed] = useState(false);

  useEffect(() => {
    if (!desktop) {
      return;
    }
    let active = true;
    const timer = window.setTimeout(() => {
      void runUpdateCheck().then((outcome) => {
        if (active && outcome.status === "available") {
          setVersion(outcome.update.info.version);
        }
      });
    }, 2500);
    return () => {
      active = false;
      window.clearTimeout(timer);
    };
  }, [desktop]);

  return (
    <UpdateBannerView
      version={dismissed ? null : version}
      onDismiss={() => setDismissed(true)}
    />
  );
}
