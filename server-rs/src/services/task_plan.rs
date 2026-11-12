//! 任务锚点校验。
//!
//! 主持人输出与手工创建 API 共用同一套边界：目标非空，至少一条验收标准，
//! 至少一条验证方式；数组只接受非空字符串并去除首尾空白。

use serde_json::Value;

use crate::domain::task::TaskPlan;

pub const MAX_PLAN_GOAL_CHARS: usize = 2000;
pub const MAX_PLAN_ACCEPTANCE_CRITERIA: usize = 10;
pub const MAX_PLAN_AC_CHARS: usize = 500;
pub const MAX_PLAN_VERIFICATION: usize = 5;
pub const MAX_PLAN_VERIFICATION_CHARS: usize = 1000;
pub const MAX_PLAN_SCOPE: usize = 10;
pub const MAX_PLAN_SCOPE_CHARS: usize = 500;

pub fn parse_task_plan(value: &Value) -> Result<TaskPlan, String> {
    let Value::Object(map) = value else {
        return Err("plan 必须是 JSON 对象".to_string());
    };

    Ok(TaskPlan {
        goal: read_string(map.get("goal"), "plan.goal", MAX_PLAN_GOAL_CHARS)?,
        acceptance_criteria: read_string_array(
            map.get("acceptanceCriteria"),
            "plan.acceptanceCriteria",
            1,
            MAX_PLAN_ACCEPTANCE_CRITERIA,
            MAX_PLAN_AC_CHARS,
        )?,
        verification: read_string_array(
            map.get("verification"),
            "plan.verification",
            1,
            MAX_PLAN_VERIFICATION,
            MAX_PLAN_VERIFICATION_CHARS,
        )?,
        scope: match map.get("scope") {
            None => Vec::new(),
            Some(value) => read_string_array(
                Some(value),
                "plan.scope",
                0,
                MAX_PLAN_SCOPE,
                MAX_PLAN_SCOPE_CHARS,
            )?,
        },
    })
}

fn read_string(value: Option<&Value>, field: &str, max_chars: usize) -> Result<String, String> {
    let Some(text) = value.and_then(Value::as_str) else {
        return Err(format!("{field} 必须是非空字符串"));
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(format!("{field} 必须是非空字符串"));
    }
    if trimmed.chars().count() > max_chars {
        return Err(format!("{field} 不能超过 {max_chars} 字符"));
    }
    Ok(trimmed.to_string())
}

fn read_string_array(
    value: Option<&Value>,
    field: &str,
    min: usize,
    max: usize,
    max_chars: usize,
) -> Result<Vec<String>, String> {
    let Some(Value::Array(items)) = value else {
        return Err(format!("{field} 必须是字符串数组"));
    };
    let mut values = Vec::with_capacity(items.len());
    for item in items {
        values.push(read_string(Some(item), field, max_chars)?);
    }
    if values.len() < min || values.len() > max {
        return Err(format!("{field} 数量必须在 {min}..{max} 之间"));
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn trims_and_validates_plan() {
        let plan = parse_task_plan(&json!({
            "goal": "  完成功能  ",
            "acceptanceCriteria": [" 可运行 "],
            "verification": ["cargo test"],
            "scope": ["server-rs"],
        }))
        .unwrap();
        assert_eq!(plan.goal, "完成功能");
        assert_eq!(plan.acceptance_criteria, vec!["可运行"]);
        assert_eq!(plan.scope, vec!["server-rs"]);
    }

    #[test]
    fn rejects_missing_or_oversized_fields() {
        assert!(parse_task_plan(&json!({})).unwrap_err().contains("plan.goal"));
        assert!(parse_task_plan(&json!({
            "goal": "g",
            "acceptanceCriteria": [],
            "verification": ["v"],
        }))
        .unwrap_err()
        .contains("数量必须在 1..10"));
        assert!(parse_task_plan(&json!({
            "goal": "x".repeat(MAX_PLAN_GOAL_CHARS + 1),
            "acceptanceCriteria": ["a"],
            "verification": ["v"],
        }))
        .unwrap_err()
        .contains("不能超过"));
        assert!(parse_task_plan(&Value::Null).is_err());
    }
}
