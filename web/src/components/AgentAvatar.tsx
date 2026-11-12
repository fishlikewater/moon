import { avatarToneIndex, initialOf } from "./avatarIdentity";

export type AgentAvatarSize = "sm" | "md" | "lg";

interface AgentAvatarProps {
  id: number;
  name: string;
  size?: AgentAvatarSize;
  className?: string;
}

/**
 * 智能体头像：首字 + 按 id 决定的色相槽位。
 *
 * 颜色完全由 styles.css 的 `.avatar` / `.avatar-tone-*` 规则与主题决定，
 * 组件不写内联颜色，避免破坏明暗主题切换。
 */
export function AgentAvatar({
  id,
  name,
  size = "md",
  className,
}: AgentAvatarProps) {
  const sizeClass = size === "md" ? "" : ` avatar-${size}`;
  const extraClass = className ? ` ${className}` : "";

  return (
    <span
      className={`avatar${sizeClass} avatar-tone-${avatarToneIndex(id)}${extraClass}`}
      aria-hidden="true"
    >
      {initialOf(name)}
    </span>
  );
}
