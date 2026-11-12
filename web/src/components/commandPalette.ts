/** 命令面板的纯逻辑：命令项构建、过滤排序与键盘索引。 */

export type CommandGroup = "页面" | "房间" | "任务" | "智能体" | "动作";

export type CommandAction = "new-room" | "new-agent" | "new-task" | "settings";

export interface CommandItem {
  id: string;
  label: string;
  group: CommandGroup;
  /** 额外搜索词，不展示。 */
  keywords: string;
  /** 导航目标；动作项与 to 二选一。 */
  to?: string;
  action?: CommandAction;
}

export interface CommandEntities {
  rooms: ReadonlyArray<{ id: number; name: string }>;
  agents: ReadonlyArray<{ id: number; name: string }>;
  tasks: ReadonlyArray<{ id: number; title: string; roomId: number }>;
}

const GROUP_ORDER: readonly CommandGroup[] = [
  "动作",
  "页面",
  "房间",
  "任务",
  "智能体",
];

export function buildCommandItems(entities: CommandEntities): CommandItem[] {
  const pages: CommandItem[] = [
    {
      id: "page-agents",
      label: "智能体",
      group: "页面",
      keywords: "agents 智能体 列表",
      to: "/agents",
    },
    {
      id: "page-rooms",
      label: "房间",
      group: "页面",
      keywords: "rooms 房间 列表",
      to: "/rooms",
    },
    {
      id: "page-tasks",
      label: "任务",
      group: "页面",
      keywords: "tasks 任务 列表",
      to: "/tasks",
    },
    {
      id: "page-settings",
      label: "设置",
      group: "页面",
      keywords: "settings 设置 配置",
      to: "/settings",
    },
  ];
  const actions: CommandItem[] = [
    {
      id: "action-new-room",
      label: "新建房间",
      group: "动作",
      keywords: "create new room 房间",
      action: "new-room",
    },
    {
      id: "action-new-agent",
      label: "新建智能体",
      group: "动作",
      keywords: "create new agent 智能体",
      action: "new-agent",
    },
    {
      id: "action-new-task",
      label: "新建任务",
      group: "动作",
      keywords: "create new task 任务",
      action: "new-task",
    },
    {
      id: "action-settings",
      label: "打开设置",
      group: "动作",
      keywords: "settings 设置 配置",
      action: "settings",
    },
  ];
  const rooms: CommandItem[] = entities.rooms.map((room) => ({
    id: `room-${room.id}`,
    label: room.name,
    group: "房间",
    keywords: `room 房间 #${room.id}`,
    to: `/rooms/${room.id}`,
  }));
  const tasks: CommandItem[] = entities.tasks.map((task) => ({
    id: `task-${task.id}`,
    label: task.title,
    group: "任务",
    keywords: `task 任务 #${task.id}`,
    to: "/tasks",
  }));
  const agents: CommandItem[] = entities.agents.map((agent) => ({
    id: `agent-${agent.id}`,
    label: agent.name,
    group: "智能体",
    keywords: `agent 智能体 #${agent.id}`,
    to: "/agents",
  }));

  return [...actions, ...pages, ...rooms, ...tasks, ...agents];
}

function score(item: CommandItem, tokens: string[]): number | null {
  const label = item.label.toLowerCase();
  const haystack = `${item.label} ${item.keywords}`.toLowerCase();
  let total = 0;
  for (const token of tokens) {
    if (label.startsWith(token)) {
      total += 0;
    } else if (label.includes(token)) {
      total += 1;
    } else if (haystack.includes(token)) {
      total += 2;
    } else {
      return null;
    }
  }
  return total;
}

/** 空查询按「动作 → 页面 → 房间 → 任务 → 智能体」；非空按匹配分数与组序。 */
export function filterCommands(
  items: readonly CommandItem[],
  query: string,
  limit = 12,
): CommandItem[] {
  const tokens = query
    .trim()
    .toLowerCase()
    .split(/\s+/)
    .filter((token) => token !== "");
  const rank = (group: CommandGroup) => GROUP_ORDER.indexOf(group);

  if (tokens.length === 0) {
    return [...items]
      .sort((left, right) => rank(left.group) - rank(right.group))
      .slice(0, limit);
  }

  const scored: Array<{ item: CommandItem; score: number; order: number }> = [];
  items.forEach((item, order) => {
    const itemScore = score(item, tokens);
    if (itemScore !== null) {
      scored.push({ item, score: itemScore, order });
    }
  });
  scored.sort((left, right) => {
    if (left.score !== right.score) {
      return left.score - right.score;
    }
    const groupDelta = rank(left.item.group) - rank(right.item.group);
    return groupDelta !== 0 ? groupDelta : left.order - right.order;
  });
  return scored.slice(0, limit).map((entry) => entry.item);
}

/** 上下键循环；空列表返回 0。 */
export function nextIndex(current: number, length: number, delta: number): number {
  if (length <= 0) {
    return 0;
  }
  return ((current + delta) % length + length) % length;
}
