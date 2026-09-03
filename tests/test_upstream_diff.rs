//! 与上游 Python phandas 逐格对照的差分测试。
//!
//! 本测试本身不做断言，只是把全部因子算子在同一份面板上跑一遍、按
//! `算子,timestamp,symbol,值` 落盘，交给 Python 侧的同名脚本产出对照文件后逐格比较。
//! 上游实现依赖 pandas / numpy / scipy，无法在 Rust 侧复现，因此标了 `#[ignore]`，
//! 默认不参与 `cargo test`，需要时显式跑：
//!
//! ```text
//! cargo test --test test_upstream_diff -- --ignored --nocapture
//! ```
//!
//! 输出落在 `target/upstream_diff_rs.csv`。Python 侧脚本与比对工具见
//! `docs/上游能力清单与移植对照.md` 第 6 节。两侧必须读同一份
//! `tests/data/panel.csv`，改动那份数据时记得两边一起重跑。

use phandas_rs::factor::{Driver, Factor, Panel};
use std::collections::BTreeMap;

/// 两侧共用的输入面板：12 期 × 4 标的，含 NaN 洞、横截面并列、零、负数、整期全平。
const PANEL: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/panel.csv");
/// 落盘位置放在 target 下，避免污染工作区。
const OUT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/target/upstream_diff_rs.csv");

fn emit(out: &mut String, key: &str, f: &Factor) {
    for (ts, sym, v) in f.to_records() {
        if v.is_nan() {
            out.push_str(&format!("{key},{ts},{sym},nan\n"));
        } else {
            out.push_str(&format!("{key},{ts},{sym},{v:.17e}\n"));
        }
    }
}

#[test]
#[ignore = "需要配合 Python 侧脚本比对，默认不跑"]
fn dump_all_operator_values() {
    let panel = Panel::from_csv(PANEL).expect("panel.csv 可解析");
    let close = panel.factor("close").expect("close");
    let volume = panel.factor("volume").expect("volume");
    let extra = panel.factor("extra").expect("extra");
    let map: BTreeMap<String, f64> = [("AAA", 1.0), ("BBB", 1.0), ("CCC", 2.0), ("DDD", 2.0)]
        .iter()
        .map(|(s, g)| ((*s).to_string(), *g))
        .collect();
    let grp = close.group_map(&map, None);
    let cond = close.gt(10.0);

    let o = &mut String::new();

    // ---- 横截面 ----
    emit(o, "rank", &close.rank());
    emit(o, "cs_mean", &close.cs_mean());
    emit(o, "cs_median", &close.cs_median());
    emit(o, "normalize_F_0", &close.normalize(false, 0.0));
    emit(o, "normalize_T_0", &close.normalize(true, 0.0));
    emit(o, "normalize_T_1.5", &close.normalize(true, 1.5));
    emit(o, "zscore", &close.zscore());
    emit(o, "scale_1", &close.scale(1.0, None, None));
    emit(o, "scale_2_ls", &close.scale(2.0, Some(0.5), Some(0.5)));
    emit(o, "quantile_gaussian", &close.quantile(Driver::Gaussian, 1.0));
    emit(o, "quantile_uniform", &close.quantile(Driver::Uniform, 1.0));
    emit(o, "quantile_cauchy_2", &close.quantile(Driver::Cauchy, 2.0));
    emit(o, "spread_034", &close.spread(0.34).expect("pct"));
    emit(o, "spread_05", &close.spread(0.5).expect("pct"));
    emit(o, "signal", &close.signal());

    // ---- 中性化 ----
    emit(o, "vector_neut", &close.vector_neut(&volume));
    emit(o, "regression_neut1", &close.regression_neut(&[&volume]));
    emit(o, "regression_neut2", &close.regression_neut(&[&volume, &extra]));

    // ---- 分组 ----
    emit(o, "group", &grp);
    emit(o, "group_neutralize", &close.group_neutralize(&grp));
    emit(o, "group_mean", &close.group_mean(&grp));
    emit(o, "group_median", &close.group_median(&grp));
    emit(o, "group_rank", &close.group_rank(&grp));
    emit(o, "group_scale", &close.group_scale(&grp));
    emit(o, "group_zscore", &close.group_zscore(&grp));
    emit(o, "group_normalize_1", &close.group_normalize(&grp, 1.0));
    // ---- 时序 ----
    emit(o, "ts_rank_3", &close.ts_rank(3));
    emit(o, "ts_sum_3", &close.ts_sum(3));
    emit(o, "ts_product_3", &close.ts_product(3));
    emit(o, "ts_mean_3", &close.ts_mean(3));
    emit(o, "ts_median_3", &close.ts_median(3));
    emit(o, "ts_std_dev_3", &close.ts_std_dev(3));
    emit(o, "ts_min_3", &close.ts_min(3));
    emit(o, "ts_max_3", &close.ts_max(3));
    emit(o, "ts_arg_max_3", &close.ts_arg_max(3));
    emit(o, "ts_arg_min_3", &close.ts_arg_min(3));
    emit(o, "ts_count_nans_3", &close.ts_count_nans(3));
    emit(o, "ts_av_diff_3", &close.ts_av_diff(3));
    emit(o, "ts_scale_3_0", &close.ts_scale(3, 0.0));
    emit(o, "ts_scale_3_1", &close.ts_scale(3, 1.0));
    emit(o, "ts_zscore_3", &close.ts_zscore(3));
    emit(o, "ts_quantile_3_g", &close.ts_quantile(3, Driver::Gaussian));
    emit(o, "ts_quantile_3_u", &close.ts_quantile(3, Driver::Uniform));
    emit(o, "ts_kurtosis_4", &close.ts_kurtosis(4));
    emit(o, "ts_skewness_3", &close.ts_skewness(3));
    emit(o, "ts_backfill_3_1", &close.ts_backfill(3, 1).expect("k"));
    emit(o, "ts_backfill_4_2", &close.ts_backfill(4, 2).expect("k"));
    emit(o, "ts_decay_exp_3_05_T", &close.ts_decay_exp_window(3, 0.5, true).expect("f"));
    emit(o, "ts_decay_exp_3_05_F", &close.ts_decay_exp_window(3, 0.5, false).expect("f"));
    emit(o, "ts_decay_linear_3_F", &close.ts_decay_linear(3, false));
    emit(o, "ts_decay_linear_3_T", &close.ts_decay_linear(3, true));
    emit(o, "ts_step_1", &close.ts_step(1));
    emit(o, "ts_step_0", &close.ts_step(0));
    emit(o, "ts_delay_2", &close.ts_delay(2));
    emit(o, "ts_delta_2", &close.ts_delta(2));
    emit(o, "ts_corr_3", &close.ts_corr(&extra, 3));
    emit(o, "ts_covariance_3", &close.ts_covariance(&extra, 3));
    for rt in [0, 1, 2, 3, 4, 5, 6, 7] {
        emit(o, &format!("ts_regression_4_0_r{rt}"), &close.ts_regression(&[&extra], 4, 0, rt));
    }
    emit(o, "ts_regression_4_1_r2", &close.ts_regression(&[&extra], 4, 1, 2));
    emit(o, "ts_regression2_4_0_r2", &close.ts_regression(&[&extra, &volume], 4, 0, 2));
    emit(o, "ts_regression2_4_0_r8", &close.ts_regression(&[&extra, &volume], 4, 0, 8));
    emit(o, "ts_regression_4_0_r9", &close.ts_regression(&[&extra], 4, 0, 9));
    emit(o, "ts_regression_4_0_r100", &close.ts_regression(&[&extra], 4, 0, 100));
    emit(o, "ts_cv_3", &close.ts_cv(3));
    emit(o, "ts_jumpiness_3", &close.ts_jumpiness(3));
    emit(o, "ts_trend_strength_4", &close.ts_trend_strength(4));
    emit(o, "ts_vr_4_2", &close.ts_vr(4, 2).expect("k"));
    emit(o, "ts_autocorr_4_1", &close.ts_autocorr(4, 1).expect("lag"));
    emit(o, "ts_reversal_count_4", &close.ts_reversal_count(4));
    // ---- 一元数学 ----
    emit(o, "abs", &close.abs());
    emit(o, "sign", &close.sign());
    emit(o, "inverse", &close.inverse());
    emit(o, "ln", &close.ln());
    emit(o, "log_2", &close.log(Some(2.0)).expect("base"));
    emit(o, "sqrt", &close.sqrt());
    emit(o, "s_log_1p", &close.s_log_1p());

    // ---- 二元 ----
    emit(o, "add_f", &close.add(&extra));
    emit(o, "add_s", &close.add(2.0));
    emit(o, "sub_f", &close.subtract(&extra));
    emit(o, "sub_s", &close.subtract(2.0));
    emit(o, "mul_f", &close.multiply(&extra));
    emit(o, "mul_s", &close.multiply(2.0));
    emit(o, "div_f", &close.divide(&extra));
    emit(o, "div_s", &close.divide(2.0));
    emit(o, "pow_s2", &close.power(2.0));
    emit(o, "pow_f", &close.power(&extra));
    emit(o, "spow_05", &close.signed_power(0.5));
    emit(o, "spow_f", &close.signed_power(&extra));
    emit(o, "max_f", &close.maximum(&extra));
    emit(o, "max_s0", &close.maximum(0.0));
    emit(o, "min_f", &close.minimum(&extra));
    emit(o, "min_s0", &close.minimum(0.0));
    emit(o, "reverse", &close.reverse());
    emit(o, "where_f", &close.where_cond(&cond, &extra));
    emit(o, "where_s0", &close.where_cond(&cond, 0.0));

    // ---- 比较 ----
    emit(o, "lt_s", &close.lt(15.0));
    emit(o, "le_s", &close.le(15.0));
    emit(o, "gt_s", &close.gt(15.0));
    emit(o, "ge_s", &close.ge(15.0));
    emit(o, "eq_s", &close.eq_val(15.0));
    emit(o, "ne_s", &close.ne_val(15.0));
    emit(o, "lt_f", &close.lt(&extra));
    emit(o, "gt_f", &close.gt(&extra));

    // ---- 运算符重载 / 反射 ----
    emit(o, "op_add_ff", &(&close + &extra));
    emit(o, "op_sub_fs", &(&close - 2.0));
    emit(o, "op_mul_fs", &(&close * 2.0));
    emit(o, "op_div_fs", &(&close / 2.0));
    emit(o, "op_neg", &(-&close));
    emit(o, "op_radd", &(2.0 + &close));
    emit(o, "op_rsub", &(2.0 - &close));
    emit(o, "op_rmul", &(2.0 * &close));
    emit(o, "op_rtruediv", &(2.0 / &close));
    emit(o, "op_rpow", &close.scalar_power(2.0));
    emit(o, "op_pow", &close.power(2.0));
    emit(o, "op_abs", &close.abs());

    std::fs::write(OUT, &*o).expect("落盘");
    println!("落盘 {} 字节 -> {OUT}", o.len());
    println!("名字派生示例：{} | {}", close.rank().name,
             close.ts_regression(&[&extra], 4, 0, 6).name);
}
