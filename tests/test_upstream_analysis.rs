//! 与上游 Python `analysis.py` 逐格对照的差分测试。
//!
//! 与 `test_upstream_diff.rs` / `test_upstream_backtest.rs` 同一套办法：两侧读同一份
//! `tests/data/panel.csv`，构造同样的因子并跑 IC / 描述统计 / 相关矩阵，把数值落盘，
//! 再由 Python 侧脚本逐格比对。上游实现依赖 pandas / numpy / scipy，无法在 Rust 侧复现，
//! 故标 `#[ignore]`：
//!
//! ```text
//! cargo test --test test_upstream_analysis -- --ignored --nocapture
//! ```
//!
//! 因子表刻意混了两类 NaN 分布：`signal(rank(close))` / `rank(close)` 的 NaN 是**整期**的
//! （`rank()` 带 `require_no_nan`，且 2024-01-06 那期 close 四个值全等触发 `nunique() == 1`），
//! 而 `close` / `volume` 只缺单格。后者才能区分 `stats()` 的 turnover 到底是"每标的先取
//! 均值再对标的取均值"（上游）还是把全部格子池化求一个均值——整期 NaN 时两者恰好相等。
//!
//! 输出落在 `target/upstream_analysis_rs.csv`，行格式为 `组,键,值`（键由 `|` 拼接）。
//! 配套脚本见 `.omc/difftest/run_py_analysis.py` 与 `compare_analysis.py`。

use phandas_rs::analysis::{CorrMethod, FactorAnalyzer, IcMethod};
use phandas_rs::factor::{Factor, Panel};

const PANEL: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/panel.csv");
const OUT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/target/upstream_analysis_rs.csv"
);

fn num(v: f64) -> String {
    if v.is_nan() {
        "nan".to_string()
    } else {
        format!("{v:.17e}")
    }
}

#[test]
#[ignore = "需要配合 Python 侧脚本比对，默认不跑"]
fn dump_analysis_scenarios() {
    let panel = Panel::from_csv(PANEL).expect("panel.csv 可解析");
    let close = panel.factor("close").expect("close");
    let volume = panel.factor("volume").expect("volume");
    let alpha1 = close.rank().signal();
    let alpha2 = close.rank();
    let factors: Vec<&Factor> = vec![&alpha1, &alpha2, &close, &volume];
    let horizons: Vec<usize> = vec![1, 3];
    let fa = FactorAnalyzer::new(&factors, &close, Some(&horizons)).expect("非空因子表");

    let out = &mut String::new();

    for (method, label) in [
        (IcMethod::Spearman, "spearman"),
        (IcMethod::Pearson, "pearson"),
    ] {
        // IC：每因子 × 每持有期的统计与逐期序列
        for fic in fa.ic(method) {
            let name = &fa.factors()[fic.factor_index].name;
            for s in &fic.by_horizon {
                let key = format!("{label}|{name}|{}", s.horizon);
                out.push_str(&format!("ic,{key}|ic_mean,{}\n", num(s.ic_mean)));
                out.push_str(&format!("ic,{key}|ic_std,{}\n", num(s.ic_std)));
                out.push_str(&format!("ic,{key}|ir,{}\n", num(s.ir)));
                out.push_str(&format!("ic,{key}|t_stat,{}\n", num(s.t_stat)));
                out.push_str(&format!("ic,{key}|n,{}\n", num(s.ic_series.len() as f64)));
                for (ts, v) in &s.ic_series {
                    out.push_str(&format!("ic_series,{key}|{ts},{}\n", num(*v)));
                }
            }
        }
    }

    // stats：coverage / turnover / autocorr
    for fs in fa.stats() {
        let name = &fa.factors()[fs.factor_index].name;
        out.push_str(&format!("stats,{name}|coverage,{}\n", num(fs.coverage)));
        out.push_str(&format!("stats,{name}|turnover,{}\n", num(fs.turnover)));
        out.push_str(&format!("stats,{name}|autocorr,{}\n", num(fs.autocorr)));
    }

    // correlation：pearson + spearman + kendall 三口径（kendall 的对角线按 pandas 是 1.0）
    for (cm, label) in [
        (fa.correlation(CorrMethod::Pearson), "pearson"),
        (fa.correlation(CorrMethod::Spearman), "spearman"),
        (fa.correlation(CorrMethod::Kendall), "kendall"),
    ] {
        for (i, a) in cm.names().iter().enumerate() {
            for (j, b) in cm.names().iter().enumerate() {
                out.push_str(&format!("corr,{label}|{a}|{b},{}\n", num(cm.at(i, j))));
            }
        }
    }

    std::fs::write(OUT, &*out).expect("落盘");
    println!("落盘 {} 字节 -> {OUT}", out.len());
}
