//! 民用日期与天数序号的互转，供回测的年化口径与回撤时长使用。
//!
//! 本仓库的 `Factor` 把时间戳存成字符串（要求 ISO-8601），而回测有三处需要真正的
//! 日历运算：播种净值时要取"起始期前一天"、年化要用首末期的日历天数差、回撤区间
//! 要报持续天数。这里用 Howard Hinnant 的 `days_from_civil` / `civil_from_days`
//! 算法在"公历年月日"与"距 1970-01-01 的天数"之间转换，零依赖且对公元前也成立。
//!
//! 解析接受 `YYYY-MM-DD` 与 `YYYY-MM-DD HH:MM:SS`（也接受 `T` 分隔），
//! 时间部分只做原样保留，不参与运算——上游按自然日聚合，日内粒度没有意义。

/// 判断闰年。
///
/// - 入参：`y` 公历年份。
/// - 加工：能被 4 整除且（不能被 100 整除或能被 400 整除）。
/// - 出参：闰年返回 `true`。
fn is_leap(y: i64) -> bool {
    y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)
}

/// 某年某月的天数。
///
/// - 入参：`y` 年；`m` 月（1–12）。
/// - 加工：查表，2 月按闰年判断给 28 或 29。
/// - 出参：该月天数；`m` 越界时返回 0（调用方据此判非法）。
fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => if is_leap(y) { 29 } else { 28 },
        _ => 0,
    }
}

/// 公历年月日 → 距 1970-01-01 的天数（Hinnant 算法）。
///
/// - 入参：`y` / `m` / `m` 已校验过的年、月、日。
/// - 加工：把 3 月当年首以消掉闰日的特例，按 400 年一个纪元折算。
/// - 出参：天数序号，1970-01-01 为 0，之前为负。
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// 距 1970-01-01 的天数 → 公历年月日（Hinnant 算法，`days_from_civil` 的逆）。
///
/// - 入参：`z` 天数序号。
/// - 加工：同上的纪元折算反向走一遍。
/// - 出参：`(年, 月, 日)`。
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// 把时间戳切成"日期部分"与"其余部分"。
///
/// - 入参：`ts` 时间戳字符串。
/// - 加工：在首个空格或 `T` 处切开。
/// - 出参：`(日期部分, 含分隔符的剩余部分)`；没有时间部分时后者为空串。
fn split_date(ts: &str) -> (&str, &str) {
    match ts.find([' ', 'T']) {
        Some(i) => (&ts[..i], &ts[i..]),
        None => (ts, ""),
    }
}

/// 解析时间戳的日期部分为天数序号。
///
/// - 入参：`ts` 形如 `2024-01-31` 或 `2024-01-31 08:00:00` 的时间戳。
/// - 加工：切出日期部分 → 按 `-` 分三段解析 → 校验月份 1–12、日在该月范围内
///   → 转成天数序号。
/// - 出参：`Some(天数)`；格式不符或日期不存在（如 `2023-02-29`）时返回 `None`。
pub fn parse_days(ts: &str) -> Option<i64> {
    let (date, _) = split_date(ts.trim());
    let mut it = date.split('-');
    let y: i64 = it.next()?.parse().ok()?;
    let m: i64 = it.next()?.parse().ok()?;
    let d: i64 = it.next()?.parse().ok()?;
    if it.next().is_some() || !(1..=12).contains(&m) || d < 1 || d > days_in_month(y, m) {
        return None;
    }
    Some(days_from_civil(y, m, d))
}

/// 时间戳平移若干天，时间部分原样保留。
///
/// - 入参：`ts` 时间戳；`delta` 平移天数（可负）。
/// - 加工：解析日期部分 → 加 `delta` → 转回年月日 → 按 `YYYY-MM-DD` 重新格式化
///   并接回原来的时间部分。
/// - 出参：`Some(新时间戳)`；`ts` 无法解析时返回 `None`。
pub fn shift_days(ts: &str, delta: i64) -> Option<String> {
    let trimmed = ts.trim();
    let (_, rest) = split_date(trimmed);
    let (y, m, d) = civil_from_days(parse_days(trimmed)? + delta);
    Some(format!("{y:04}-{m:02}-{d:02}{rest}"))
}

/// 两个时间戳相差的日历天数。
///
/// - 入参：`from` 起始时间戳；`to` 结束时间戳。
/// - 加工：各自解析成天数序号后相减。
/// - 出参：`Some(to - from)`，`to` 更早时为负；任一侧无法解析时返回 `None`。
///   对应 Python 侧 `(index[-1] - index[0]).days`。
pub fn span_days(from: &str, to: &str) -> Option<i64> {
    Some(parse_days(to)? - parse_days(from)?)
}
