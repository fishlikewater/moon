/** 列表页 `?new=1` 入口的纯逻辑，便于直接测试。 */

type SearchParamsLike = Pick<URLSearchParams, "get">;

/** 仅当参数精确为 `new=1` 时打开新建模态框。 */
export function shouldOpenNew(params: SearchParamsLike): boolean {
  return params.get("new") === "1";
}

/** 只移除 `new`，保留其它查询参数（如房间筛选）。 */
export function withoutNewParam(params: URLSearchParams): URLSearchParams {
  const next = new URLSearchParams(params);
  next.delete("new");
  return next;
}
