/** 探测能力标签组：模型 / 权限模式各一组，当前项高亮并带「当前」徽标。 */
export function CapabilityChips({
  label,
  values,
  currentValue,
}: {
  label: string;
  values: string[];
  currentValue: string | null;
}) {
  const entries: Array<{ value: string; current: boolean }> = [];
  for (const value of values) {
    entries.push({ value, current: value === currentValue });
  }
  // 当前值不在宣告列表时也展示出来（带当前徽标），避免「当前 X」信息丢失。
  if (currentValue !== null && !values.includes(currentValue)) {
    entries.push({ value: currentValue, current: true });
  }
  if (entries.length === 0) {
    return (
      <div className="capability-group">
        <span className="capability-label">{label}</span>
        <span className="capability-empty">未宣告</span>
      </div>
    );
  }
  return (
    <div className="capability-group">
      <span className="capability-label">{label}</span>
      <div className="capability-chips">
        {entries.map(({ value, current }) => (
          <span
            key={value}
            className={"capability-chip" + (current ? " is-current" : "")}
          >
            {value}
            {current && <span className="capability-current-tag">当前</span>}
          </span>
        ))}
      </div>
    </div>
  );
}
