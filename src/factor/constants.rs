//! 常量与内置分组定义，对应 Python 侧 `constants.py`。

use std::collections::BTreeMap;

/// 数值零阈值，对应 Python 侧 `constants.EPSILON`。
pub const EPSILON: f64 = 1e-10;
/// 浮点比较容差，对应 Python 侧 `constants.TOLERANCE_FLOAT`。
pub const TOLERANCE_FLOAT: f64 = 1e-6;
/// 最小二乘条件数上限，对应 Python 侧 `constants.MATRIX_COND_THRESHOLD`。
pub const MATRIX_COND_THRESHOLD: f64 = 1e10;

/// 美元中性信号的多头权重和，对应 Python 侧 `constants.SIGNAL_LONG_SUM`。
pub const SIGNAL_LONG_SUM: f64 = 0.5;
/// 美元中性信号的空头权重和，对应 Python 侧 `constants.SIGNAL_SHORT_SUM`。
pub const SIGNAL_SHORT_SUM: f64 = -0.5;
/// 判定是否为美元中性信号时的绝对容差，对应 Python 侧 `constants.SIGNAL_TOLERANCE`。
pub const SIGNAL_TOLERANCE: f64 = 1e-2;

/// 内置分组定义，对应 Python 侧 `constants.GROUP_DEFINITIONS`。
///
/// - 入参：`name` 分组方案名（`"SECTOR_L1_L2"` 或 `"DAPP_ACTIVITY"`）。
/// - 加工：查表取出该方案的 symbol → 组号对照。
/// - 出参：`Some(映射表)`；方案名未知时返回 `None`。
pub fn group_definitions(name: &str) -> Option<BTreeMap<String, f64>> {
    let pairs: &[(&str, f64)] = match name {
        // Group 1: L1，Group 2: L2
        "SECTOR_L1_L2" => &[
            ("ETH", 1.0),
            ("SOL", 1.0),
            ("SUI", 1.0),
            ("ARB", 2.0),
            ("OP", 2.0),
            ("POL", 2.0),
        ],
        // Group 1: High TVL/Dapps，Group 2: Growth/Alt
        "DAPP_ACTIVITY" => &[
            ("POL", 1.0),
            ("ETH", 1.0),
            ("ARB", 1.0),
            ("OP", 1.0),
            ("SUI", 2.0),
            ("SOL", 2.0),
        ],
        _ => return None,
    };
    Some(pairs.iter().map(|(s, g)| ((*s).to_string(), *g)).collect())
}
