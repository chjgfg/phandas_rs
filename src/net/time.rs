//! UTC 时间换算：epoch 毫秒 ↔ 时间戳字符串，以及 OKX 签名要的 ISO-8601。
//!
//! 不引 `chrono` / `time`：日期部分复用 [`crate::backtest::date`] 里已有的 Hinnant
//! `days_from_civil` / `civil_from_days`，日内部分只是对一天的毫秒数做整除取余。
//!
//! **全程 UTC**，不做时区转换。上游 `data.py` 同样没有任何时区参数：`start_date` /
//! `end_date` 一律按当日 `00:00:00Z` 解释（上游 `parse8601(f'{date}T00:00:00Z')`），
//! Binance 的 `openTime` 本身也是 UTC 毫秒。

use std::time::{SystemTime, UNIX_EPOCH};

use crate::backtest::date::{civil_from_days, parse_days};
// 只有 data 用得到自然月进位，故连它依赖的两个日期函数一起按 feature 引入
#[cfg(feature = "data")]
use crate::backtest::date::{days_from_civil, days_in_month};

/// 一天的毫秒数。
const MS_PER_DAY: i64 = 86_400_000;

/// 把 epoch 毫秒拆成 UTC 的年月日时分秒毫秒。
///
/// - 入参：`ms` 距 1970-01-01T00:00:00Z 的毫秒数，可为负。
/// - 加工：用 `div_euclid` / `rem_euclid` 做向负无穷取整的除法（1970 之前也成立）
///   拆出天数与日内余量 → 天数交给 [`civil_from_days`]，余量逐级整除。
/// - 出参：`(年, 月, 日, 时, 分, 秒, 毫秒)`。
pub(crate) fn split_millis(ms: i64) -> (i64, i64, i64, i64, i64, i64, i64) {
    let days = ms.div_euclid(MS_PER_DAY);
    let rem = ms.rem_euclid(MS_PER_DAY);
    let (y, m, d) = civil_from_days(days);
    (
        y,
        m,
        d,
        rem / 3_600_000,
        rem / 60_000 % 60,
        rem / 1_000 % 60,
        rem % 1_000,
    )
}

/// 星期几，周一为 0。
///
/// - 入参：`ms` epoch 毫秒。
/// - 加工：1970-01-01 是星期四（周一记 0 时为 3），故 `(天数 + 3) mod 7`。
/// - 出参：`0`（周一）到 `6`（周日）。日历列的 `dayofweek` 是它 `+ 1`
///   （上游 `date.dayofweek + 1`，周一记 1）。
#[cfg(feature = "data")]
pub(crate) fn weekday_mon0(ms: i64) -> i64 {
    (ms.div_euclid(MS_PER_DAY) + 3).rem_euclid(7)
}

/// 截断到当日 `00:00:00Z`。
///
/// - 入参：`ms` epoch 毫秒。
/// - 加工：按一天的毫秒数向下取整。
/// - 出参：当日零点的毫秒。VWAP 按自然日聚合要用它。
#[cfg(feature = "data")]
pub(crate) fn floor_to_day(ms: i64) -> i64 {
    ms.div_euclid(MS_PER_DAY) * MS_PER_DAY
}

/// 在 epoch 毫秒上加减若干个**自然月**，日内时刻原样保留。
///
/// - 入参：`ms` epoch 毫秒；`months` 月数（可负）。
/// - 加工：拆出年月日与日内余量 → 年月按 12 进制进位 → 目标月天数不足时把日夹到月末
///   （`1-31` 加一月落到 `2-28` / `2-29`）→ 重新组回毫秒。
/// - 出参：新的 epoch 毫秒。月线网格要用它——`1M` 的步长不是常数，
///   Binance 的月线开在每月 1 日 `00:00:00Z`。
#[cfg(feature = "data")]
pub(crate) fn add_months(ms: i64, months: i64) -> i64 {
    let (y, m, d, ..) = split_millis(ms);
    let rem = ms.rem_euclid(MS_PER_DAY);
    let total = (y * 12 + (m - 1)) + months;
    let (ny, nm) = (total.div_euclid(12), total.rem_euclid(12) + 1);
    let nd = d.min(days_in_month(ny, nm));
    days_from_civil(ny, nm, nd) * MS_PER_DAY + rem
}

/// `YYYY-MM-DD` → 当日 `00:00:00Z` 的 epoch 毫秒。
///
/// - 入参：`date` 日期字符串。也接受 `YYYY-MM-DD HH:MM:SS` / 带 `T` 分隔的形式，
///   **时间部分一律忽略**——对齐上游 `parse8601(f'{date}T00:00:00Z')` 的语义
///   （上游 `fetch_vwap` 传的正是带时间的串，靠 `re.search` 的宽松匹配落到同一结果）。
/// - 加工：交给 [`parse_days`] 解析日期部分，再乘一天的毫秒数。
/// - 出参：`Some(毫秒)`；格式不符或日期不存在（如 `2023-02-29`）时返回 `None`。
pub fn date_to_millis(date: &str) -> Option<i64> {
    Some(parse_days(date)? * MS_PER_DAY)
}

/// epoch 毫秒 → `YYYY-MM-DD`。
///
/// - 入参：`ms` epoch 毫秒。
/// - 加工：拆出年月日，丢掉日内部分。
/// - 出参：日期字符串。日线及更粗的周期用它当 `Factor` 的时间索引——与上游 pandas
///   在「全是午夜」时写出的形式一致。
pub fn millis_to_date(ms: i64) -> String {
    let (y, m, d, ..) = split_millis(ms);
    format!("{y:04}-{m:02}-{d:02}")
}

/// epoch 毫秒 → `YYYY-MM-DD HH:MM:SS`。
///
/// - 入参：`ms` epoch 毫秒。
/// - 加工：拆出年月日时分秒，丢掉毫秒。
/// - 出参：日期时间字符串。日内周期用它当时间索引；空格分隔与 [`crate::backtest::date`]
///   的解析口径一致（那边同时接受空格与 `T`），且字典序与时间序一致。
pub fn millis_to_datetime(ms: i64) -> String {
    let (y, m, d, h, mi, s, _) = split_millis(ms);
    format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02}:{s:02}")
}

/// 当前时刻的 ISO-8601（带毫秒、UTC、以 `Z` 结尾），OKX 的 `OK-ACCESS-TIMESTAMP` 要这个格式。
///
/// - 入参：无。
/// - 加工：`SystemTime::now()` 相对 epoch 求毫秒（系统时钟早于 1970 时取负值），再拆开格式化。
/// - 出参：形如 `2026-09-04T12:34:56.789Z` 的字符串。OKX 要求它与服务器时间相差不超过 30 秒，
///   故本机时钟必须是对的——签名被拒时先查这里。
pub fn now_iso8601_millis() -> String {
    let now = SystemTime::now();
    let ms = match now.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(e) => -(e.duration().as_millis() as i64),
    };
    let (y, m, d, h, mi, s, milli) = split_millis(ms);
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{milli:03}Z")
}
