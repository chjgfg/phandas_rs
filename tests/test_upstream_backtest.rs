//! 与上游 Python `backtest.py` 逐格对照的差分测试。
//!
//! 与 `test_upstream_diff.rs` 同一套办法：两侧读同一份 `tests/data/panel.csv`、跑同样三个
//! 场景，把净值、收益率、11 项指标、成交笔数、回撤区间落盘，再由 Python 侧脚本逐格比对。
//! 上游实现依赖 pandas / numpy / scipy，无法在 Rust 侧复现，故标 `#[ignore]`：
//!
//! ```text
//! cargo test --test test_upstream_backtest -- --ignored --nocapture
//! ```
//!
//! 输出落在 `target/upstream_backtest_rs.csv`。场景 `B_signal_fr` 预期与上游不同——
//! 上游在 `full_rebalance` 模式下同一天记两条净值，详见 `src/backtest/mod.rs` 的偏差说明。

use phandas_rs::backtest::Backtester;
use phandas_rs::factor::{Factor, Panel};

const PANEL: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/panel.csv");
const OUT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/target/upstream_backtest_rs.csv");

const METRIC_KEYS: [&str; 11] = [
    "total_return", "annual_return", "annual_volatility", "sharpe_ratio", "sortino_ratio",
    "calmar_ratio", "max_drawdown", "linearity", "var_95", "cvar", "psr",
];

fn num(v: f64) -> String {
    if v.is_nan() {
        "nan".to_string()
    } else {
        format!("{v:.17e}")
    }
}

fn dump(out: &mut String, scenario: &str, price: &Factor, strategy: &Factor, full_rebalance: bool) {
    let bt = Backtester::new(price, strategy)
        .transaction_cost(0.0003, 0.0003)
        .initial_capital(1000.0)
        .full_rebalance(full_rebalance)
        .run()
        .expect("数据足够")
        .calculate_metrics(0.03);

    for (i, (ts, v)) in bt.equity().iter().enumerate() {
        out.push_str(&format!("{scenario},equity,{i:03}_{ts},{}\n", num(*v)));
    }
    for (i, (ts, v)) in bt.returns().iter().enumerate() {
        out.push_str(&format!("{scenario},returns,{i:03}_{ts},{}\n", num(*v)));
    }
    let m = bt.metrics().expect("已算指标");
    let values = [
        m.total_return, m.annual_return, m.annual_volatility, m.sharpe_ratio, m.sortino_ratio,
        m.calmar_ratio, m.max_drawdown, m.linearity, m.var_95, m.cvar, m.psr,
    ];
    for (k, v) in METRIC_KEYS.iter().zip(values.iter()) {
        out.push_str(&format!("{scenario},metric,{k},{}\n", num(*v)));
    }
    out.push_str(&format!(
        "{scenario},count,trades,{}\n",
        num(bt.trades().len() as f64)
    ));
    for (i, p) in bt.drawdown_periods().iter().enumerate() {
        out.push_str(&format!("{scenario},dd,{i:02}_start_{},0\n", p.start));
        out.push_str(&format!("{scenario},dd,{i:02}_end_{},0\n", p.end));
        out.push_str(&format!("{scenario},dd,{i:02}_depth,{}\n", num(p.depth)));
        out.push_str(&format!("{scenario},dd,{i:02}_days,{}\n", num(p.duration_days as f64)));
    }
    println!(
        "{scenario:<12} 净值 {} 条，成交 {} 笔，回撤段 {} 段",
        bt.equity().len(),
        bt.trades().len(),
        bt.drawdown_periods().len()
    );
}

#[test]
#[ignore = "需要配合 Python 侧脚本比对，默认不跑"]
fn dump_backtest_scenarios() {
    let panel = Panel::from_csv(PANEL).expect("panel.csv 可解析");
    let close = panel.factor("close").expect("close");
    let extra = panel.factor("extra").expect("extra");
    let signal = close.rank().signal();
    let ranked = close.rank();

    let out = &mut String::new();
    dump(out, "A_signal", &extra, &signal, false);
    dump(out, "B_signal_fr", &extra, &signal, true);
    dump(out, "C_rank", &extra, &ranked, false);

    std::fs::write(OUT, &*out).expect("落盘");
    println!("落盘 {} 字节 -> {OUT}", out.len());
}
