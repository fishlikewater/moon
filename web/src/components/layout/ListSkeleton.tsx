export type ListSkeletonVariant = "agent" | "room" | "task";

interface ListSkeletonProps {
  variant: ListSkeletonVariant;
  count?: number;
}

/**
 * 列表加载骨架：只呈现占位结构，不带数据、不发起请求。
 *
 * `role="status"` 与可读文案保证屏幕阅读器知道列表正在加载；
 * 动画在 `prefers-reduced-motion` 下由全局样式停用。
 */
export function ListSkeleton({ variant, count = 6 }: ListSkeletonProps) {
  const rows = Math.max(1, Math.trunc(count));

  return (
    <div
      className={`skeleton-list skeleton-${variant}`}
      role="status"
      aria-label="正在加载列表"
    >
      <span className="visually-hidden">正在加载列表…</span>
      {Array.from({ length: rows }, (_, index) => (
        <span className="skeleton-row" key={index} aria-hidden="true" />
      ))}
    </div>
  );
}
