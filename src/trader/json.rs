//! OKX 响应的字段提取。
//!
//! OKX v5 有两条贯穿全部端点的约定，这里集中处理，免得每个调用点各写一遍：
//!
//! 1. **数值字段一律是字符串**（`"ctVal": "0.01"`、`"pos": "-3"`）。
//! 2. **「不适用」用空串表示**而不是 `null`——刚建的仓位 `avgPx` 就是 `""`。
//!
//! 所以取数值必须「取字符串再 parse」，且要把空串当缺失。上游用 `_safe_float` 做这件事，
//! 但只在两个方法里用了；`get_positions` / `get_ticker` 用的是裸 `float()`，遇到空串会抛
//! 未捕获的 `ValueError`（见 [`super`] 的模块文档）。本仓库全程走这里。

use serde_json::Value;

/// 取字符串字段；缺失或类型不符时给空串。
///
/// - 入参：`v` JSON 对象；`key` 字段名。
/// - 加工：`v[key]` 当字符串取。
/// - 出参：字符串切片；缺失时为 `""`。
pub(crate) fn s_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

/// 取数值字段，缺失 / 空串 / 解析失败都给 `None`。
///
/// - 入参：`v` JSON 对象；`key` 字段名。
/// - 加工：先按字符串取再 `parse`，同时兼容 OKX 偶尔直接给数字的情形。
/// - 出参：`Some(数值)`；取不到时 `None`——调用方据此决定是报错还是取默认值。
pub(crate) fn opt_f(v: &Value, key: &str) -> Option<f64> {
    match v.get(key) {
        Some(Value::String(s)) if !s.is_empty() => s.parse().ok(),
        Some(Value::Number(n)) => n.as_f64(),
        _ => None,
    }
}

/// 取数值字段，取不到就给 `0.0`。对应上游 `_safe_float`。
///
/// - 入参：`v` JSON 对象；`key` 字段名。
/// - 加工：[`opt_f`] 的结果兜个默认值。
/// - 出参：数值；缺失 / 空串 / 解析失败时为 `0.0`。
pub(crate) fn f_of(v: &Value, key: &str) -> f64 {
    opt_f(v, key).unwrap_or(0.0)
}
