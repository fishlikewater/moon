import type { ReactNode } from "react";

interface EmptyStateProps {
  title: string;
  description?: string;
  action?: ReactNode;
}

/** 列表/面板统一空态。 */
export function EmptyState({ title, description, action }: EmptyStateProps) {
  return (
    <div className="empty-state">
      <p>{title}</p>
      {description !== undefined && (
        <p className="muted small">{description}</p>
      )}
      {action}
    </div>
  );
}
