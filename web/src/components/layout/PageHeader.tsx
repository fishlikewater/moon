import type { ReactNode } from "react";
import { Link } from "react-router-dom";

interface PageHeaderProps {
  title: ReactNode;
  /** 标题下的说明文字；与 `meta` 二选一或同时使用。 */
  subtitle?: ReactNode;
  backTo?: string;
  backLabel?: string;
  /** 路径、统计等附加信息，放在标题下方。 */
  meta?: ReactNode;
  actions?: ReactNode;
  className?: string;
  actionsClassName?: string;
}

/** 页面统一页头：返回链接、标题、说明与右侧动作。 */
export function PageHeader({
  title,
  subtitle,
  backTo,
  backLabel = "返回",
  meta,
  actions,
  className,
  actionsClassName,
}: PageHeaderProps) {
  return (
    <header className={className ? `page-head ${className}` : "page-head"}>
      <div className="page-head-main">
        {backTo !== undefined && (
          <Link className="back-link" to={backTo}>
            ← {backLabel}
          </Link>
        )}
        <h1>{title}</h1>
        {subtitle !== undefined && <p className="page-sub">{subtitle}</p>}
        {meta}
      </div>
      {actions !== undefined && (
        <div
          className={
            actionsClassName
              ? `page-head-actions ${actionsClassName}`
              : "page-head-actions"
          }
        >
          {actions}
        </div>
      )}
    </header>
  );
}
