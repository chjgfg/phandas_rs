//! `factor` 模块的集成测试，覆盖 `Panel`、`Factor` 与全部因子算子。

use std::collections::BTreeMap;

use phandas_rs::factor::numeric::{nanstd, norm_ppf};
use phandas_rs::factor::*;

/// 4 期 × 3 标的的小面板，覆盖含 NaN 的场景。
fn sample_panel() -> Panel {
    let csv = "\
timestamp,symbol,open,high,low,close,volume
2024-01-01,AAA,10,11,9,10,100
2024-01-01,BBB,20,21,19,20,200
2024-01-01,CCC,30,31,29,30,300
2024-01-02,AAA,10,12,10,11,110
2024-01-02,BBB,20,22,18,19,190
2024-01-02,CCC,30,33,29,32,330
2024-01-03,AAA,11,13,11,12,120
2024-01-03,BBB,19,20,17,18,180
2024-01-03,CCC,32,35,31,34,340
2024-01-04,AAA,12,14,12,14,130
2024-01-04,BBB,18,19,16,16,170
2024-01-04,CCC,34,36,33,35,350
";
    Panel::from_csv_str(csv).expect("示例面板可解析")
}

fn assert_close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-9, "期望 {b} 实际 {a}");
}

#[test]
fn panel_parses_index_and_columns() {
    let panel = sample_panel();
    assert_eq!(panel.timestamps().len(), 4);
    assert_eq!(panel.symbols(), ["AAA", "BBB", "CCC"]);
    assert_eq!(
        panel.column_names(),
        ["close", "high", "low", "open", "volume"]
    );
    assert_eq!(panel.len(), 12);
}

#[test]
fn factor_extraction_keeps_values() {
    let close = sample_panel().factor("close").expect("close 列存在");
    assert_eq!(close.name, "close");
    assert_close(close.at(0, 0), 10.0);
    assert_close(close.at(3, 2), 35.0);
    assert_close(close.get("2024-01-02", "CCC").expect("存在"), 32.0);
}

#[test]
fn missing_column_is_reported() {
    let err = sample_panel().factor("vwap").expect_err("列不存在");
    assert!(err.contains("vwap"));
}

#[test]
fn rank_is_cross_sectional_percentile() {
    let close = sample_panel().factor("close").expect("close 列存在");
    let r = close.rank();
    // 首期 close 为 10 / 20 / 30，名次 1/3、2/3、3/3
    assert_close(r.at(0, 0), 1.0 / 3.0);
    assert_close(r.at(0, 1), 2.0 / 3.0);
    assert_close(r.at(0, 2), 1.0);
    assert_eq!(r.name, "rank(close)");
}

#[test]
fn rank_yields_nan_when_row_is_flat() {
    let f = Factor::from_records(
        vec![
            ("2024-01-01".into(), "AAA".into(), 5.0),
            ("2024-01-01".into(), "BBB".into(), 5.0),
        ],
        "flat",
    );
    assert!(f.rank().at(0, 0).is_nan());
}

#[test]
fn ts_delay_shifts_within_symbol() {
    let close = sample_panel().factor("close").expect("close 列存在");
    let d = close.ts_delay(1);
    assert!(d.at(0, 0).is_nan());
    assert_close(d.at(1, 0), 10.0);
    assert_close(d.at(3, 1), 18.0);
    assert_eq!(d.name, "ts_delay(close,1)");
}

#[test]
fn ts_delta_matches_manual_difference() {
    let close = sample_panel().factor("close").expect("close 列存在");
    let d = close.ts_delta(2);
    assert!(d.at(1, 0).is_nan());
    // AAA: 12 - 10
    assert_close(d.at(2, 0), 2.0);
    // BBB: 16 - 19
    assert_close(d.at(3, 1), -3.0);
}

#[test]
fn ts_mean_requires_full_clean_window() {
    let close = sample_panel().factor("close").expect("close 列存在");
    let m = close.ts_mean(3);
    assert!(m.at(0, 0).is_nan());
    assert!(m.at(1, 0).is_nan());
    // AAA 前三期收盘 10 / 11 / 12
    assert_close(m.at(2, 0), 11.0);
    assert_close(m.at(3, 0), (11.0 + 12.0 + 14.0) / 3.0);
}

#[test]
fn ts_std_dev_uses_sample_ddof() {
    let f = Factor::from_records(
        vec![
            ("2024-01-01".into(), "AAA".into(), 1.0),
            ("2024-01-02".into(), "AAA".into(), 2.0),
            ("2024-01-03".into(), "AAA".into(), 3.0),
        ],
        "x",
    );
    // 样本标准差（ddof = 1）为 1.0
    assert_close(f.ts_std_dev(3).at(2, 0), 1.0);
}

#[test]
fn ts_arg_max_counts_distance_from_now() {
    let f = Factor::from_records(
        vec![
            ("2024-01-01".into(), "AAA".into(), 1.0),
            ("2024-01-02".into(), "AAA".into(), 9.0),
            ("2024-01-03".into(), "AAA".into(), 3.0),
        ],
        "x",
    );
    // 最大值 9 出现在中间一期，距当期 1 期
    assert_close(f.ts_arg_max(3).at(2, 0), 1.0);
    assert_close(f.ts_arg_min(3).at(2, 0), 2.0);
}

#[test]
fn ts_rank_scores_latest_value_in_window() {
    let f = Factor::from_records(
        vec![
            ("2024-01-01".into(), "AAA".into(), 1.0),
            ("2024-01-02".into(), "AAA".into(), 2.0),
            ("2024-01-03".into(), "AAA".into(), 3.0),
            ("2024-01-04".into(), "AAA".into(), 0.0),
        ],
        "x",
    );
    let r = f.ts_rank(3);
    // 当期为窗口内最大 → 3/3
    assert_close(r.at(2, 0), 1.0);
    // 当期为窗口内最小 → 1/3
    assert_close(r.at(3, 0), 1.0 / 3.0);
}

#[test]
fn ts_sum_and_product_compose() {
    let close = sample_panel().factor("close").expect("close 列存在");
    assert_close(close.ts_sum(2).at(1, 0), 21.0);
    assert_close(close.ts_product(2).at(1, 0), 110.0);
}

#[test]
fn ts_count_nans_counts_within_window() {
    let f = Factor::from_records(
        vec![
            ("2024-01-01".into(), "AAA".into(), f64::NAN),
            ("2024-01-02".into(), "AAA".into(), 2.0),
            ("2024-01-03".into(), "AAA".into(), f64::NAN),
        ],
        "x",
    );
    let c = f.ts_count_nans(3);
    // 首期窗口内全为 NaN，pandas 的 min_periods=1 语义下输出 NaN
    assert!(c.at(0, 0).is_nan());
    assert_close(c.at(1, 0), 1.0);
    assert_close(c.at(2, 0), 2.0);
}

#[test]
fn ts_backfill_fills_from_latest_valid() {
    let f = Factor::from_records(
        vec![
            ("2024-01-01".into(), "AAA".into(), 5.0),
            ("2024-01-02".into(), "AAA".into(), f64::NAN),
            ("2024-01-03".into(), "AAA".into(), f64::NAN),
        ],
        "x",
    );
    let b = f.ts_backfill(3, 1).expect("k 合法");
    assert_close(b.at(0, 0), 5.0);
    assert_close(b.at(1, 0), 5.0);
    assert_close(b.at(2, 0), 5.0);
    assert!(f.ts_backfill(3, 0).is_err());
}

#[test]
fn ts_corr_of_identical_series_is_one() {
    let close = sample_panel().factor("close").expect("close 列存在");
    let c = close.ts_corr(&close, 3);
    assert_close(c.at(2, 0), 1.0);
    assert_close(c.at(3, 2), 1.0);
}

#[test]
fn ts_corr_detects_inverse_relation() {
    let close = sample_panel().factor("close").expect("close 列存在");
    let c = close.ts_corr(&close.reverse(), 3);
    assert_close(c.at(2, 0), -1.0);
}

#[test]
fn ts_covariance_matches_manual_formula() {
    let f = Factor::from_records(
        vec![
            ("2024-01-01".into(), "AAA".into(), 1.0),
            ("2024-01-02".into(), "AAA".into(), 2.0),
            ("2024-01-03".into(), "AAA".into(), 3.0),
        ],
        "x",
    );
    // 与自身的样本协方差即样本方差 1.0
    assert_close(f.ts_covariance(&f, 3).at(2, 0), 1.0);
}

#[test]
fn ts_regression_recovers_known_slope() {
    let y = Factor::from_records(
        vec![
            ("2024-01-01".into(), "AAA".into(), 3.0),
            ("2024-01-02".into(), "AAA".into(), 5.0),
            ("2024-01-03".into(), "AAA".into(), 7.0),
            ("2024-01-04".into(), "AAA".into(), 9.0),
        ],
        "y",
    );
    let x = Factor::from_records(
        vec![
            ("2024-01-01".into(), "AAA".into(), 1.0),
            ("2024-01-02".into(), "AAA".into(), 2.0),
            ("2024-01-03".into(), "AAA".into(), 3.0),
            ("2024-01-04".into(), "AAA".into(), 4.0),
        ],
        "x",
    );
    // y = 1 + 2x：截距 1、斜率 2、R² 为 1、残差为 0
    assert_close(y.ts_regression(&[&x], 4, 0, 1).at(3, 0), 1.0);
    assert_close(y.ts_regression(&[&x], 4, 0, 2).at(3, 0), 2.0);
    assert_close(y.ts_regression(&[&x], 4, 0, 6).at(3, 0), 1.0);
    assert_close(y.ts_regression(&[&x], 4, 0, 0).at(3, 0), 0.0);
}

#[test]
fn ts_trend_strength_is_one_for_linear_series() {
    let f = Factor::from_records(
        vec![
            ("2024-01-01".into(), "AAA".into(), 1.0),
            ("2024-01-02".into(), "AAA".into(), 2.0),
            ("2024-01-03".into(), "AAA".into(), 3.0),
            ("2024-01-04".into(), "AAA".into(), 4.0),
        ],
        "x",
    );
    assert_close(f.ts_trend_strength(4).at(3, 0), 1.0);
}

#[test]
fn signal_is_dollar_neutral() {
    let close = sample_panel().factor("close").expect("close 列存在");
    let s = close.signal();
    let row = s.row(0);
    let long_sum: f64 = row.iter().filter(|x| **x > 0.0).sum();
    let short_sum: f64 = row.iter().filter(|x| **x < 0.0).sum();
    assert_close(long_sum, 0.5);
    assert_close(short_sum, -0.5);
    assert_close(row.iter().sum::<f64>(), 0.0);
}

#[test]
fn spread_marks_extremes_only() {
    let close = sample_panel().factor("close").expect("close 列存在");
    let s = close.spread(0.34).expect("pct 合法");
    // 3 个标的、pct = 0.34 → 各取 1 个：最低 -0.5、最高 +0.5、中间 0
    assert_close(s.at(0, 0), -0.5);
    assert_close(s.at(0, 1), 0.0);
    assert_close(s.at(0, 2), 0.5);
    assert!(close.spread(0.0).is_err());
    assert!(close.spread(1.0).is_err());
}

#[test]
fn scale_normalizes_absolute_sum() {
    let close = sample_panel().factor("close").expect("close 列存在");
    let s = close.scale(1.0, None, None);
    let total: f64 = s.row(0).iter().map(|x| x.abs()).sum();
    assert_close(total, 1.0);
}

#[test]
fn zscore_centers_and_standardizes() {
    let close = sample_panel().factor("close").expect("close 列存在");
    let z = close.zscore();
    let row = z.row(0);
    assert_close(row.iter().sum::<f64>(), 0.0);
    assert_close(nanstd(row, 1), 1.0);
    assert_eq!(z.name, "zscore(close)");
}

#[test]
fn normalize_limit_clips_output() {
    let close = sample_panel().factor("close").expect("close 列存在");
    let n = close.normalize(false, 5.0);
    assert!(n.values().iter().all(|v| v.abs() <= 5.0 + 1e-12));
}

#[test]
fn quantile_is_monotone_in_rank() {
    let close = sample_panel().factor("close").expect("close 列存在");
    let q = close.quantile(Driver::Gaussian, 1.0);
    // 收盘价升序 → 映射后同样升序
    assert!(q.at(0, 0) < q.at(0, 1));
    assert!(q.at(0, 1) < q.at(0, 2));
}

#[test]
fn norm_ppf_matches_known_quantiles() {
    assert!((norm_ppf(0.5)).abs() < 1e-12);
    assert!((norm_ppf(0.975) - 1.959_963_984_540_054).abs() < 1e-6);
    assert!((norm_ppf(0.025) + 1.959_963_984_540_054).abs() < 1e-6);
    assert!((norm_ppf(0.001) + 3.090_232_306_167_813).abs() < 1e-6);
}

#[test]
fn vector_neut_removes_projection() {
    let close = sample_panel().factor("close").expect("close 列存在");
    let volume = sample_panel().factor("volume").expect("volume 列存在");
    let neutral = close.rank().vector_neut(&volume.rank());
    // 中性化后与中性化目标的内积应接近 0
    let y = volume.rank();
    let dot: f64 = (0..neutral.n_symbols())
        .map(|si| neutral.at(0, si) * y.at(0, si))
        .filter(|v| !v.is_nan())
        .sum();
    assert!(dot.abs() < 1e-9, "残留投影 {dot}");
}

#[test]
fn vector_neut_keeps_input_when_target_is_flat() {
    let x = Factor::from_records(
        vec![
            ("2024-01-01".into(), "AAA".into(), 1.0),
            ("2024-01-01".into(), "BBB".into(), 2.0),
        ],
        "x",
    );
    let y = Factor::from_records(
        vec![
            ("2024-01-01".into(), "AAA".into(), 3.0),
            ("2024-01-01".into(), "BBB".into(), 3.0),
        ],
        "y",
    );
    let out = x.vector_neut(&y);
    assert_close(out.at(0, 0), 1.0);
    assert_close(out.at(0, 1), 2.0);
}

#[test]
fn regression_neut_residuals_are_orthogonal() {
    let close = sample_panel().factor("close").expect("close 列存在");
    let volume = sample_panel().factor("volume").expect("volume 列存在");
    let resid = close.regression_neut(&[&volume]);
    // 残差与自变量、以及与常数项（即残差和）都应正交
    let sum: f64 = (0..resid.n_symbols()).map(|si| resid.at(0, si)).sum();
    let dot: f64 = (0..resid.n_symbols())
        .map(|si| resid.at(0, si) * volume.at(0, si))
        .sum();
    assert!(sum.abs() < 1e-9, "残差和 {sum}");
    assert!(dot.abs() < 1e-6, "残差与自变量内积 {dot}");
}

#[test]
fn group_neutralize_removes_group_mean() {
    let close = sample_panel().factor("close").expect("close 列存在");
    let mut mapping = BTreeMap::new();
    mapping.insert("AAA".to_string(), 1.0);
    mapping.insert("BBB".to_string(), 1.0);
    mapping.insert("CCC".to_string(), 2.0);
    let groups = close.group_map(&mapping, None);
    let out = close.group_neutralize(&groups);
    // 组 1 为 AAA / BBB，首期 10 与 20，去均值后 ±5
    assert_close(out.at(0, 0), -5.0);
    assert_close(out.at(0, 1), 5.0);
    // 组 2 只有 CCC，去均值后为 0
    assert_close(out.at(0, 2), 0.0);
}

#[test]
fn group_named_uses_builtin_definitions() {
    let f = Factor::from_records(
        vec![
            ("2024-01-01".into(), "ETH".into(), 1.0),
            ("2024-01-01".into(), "ARB".into(), 2.0),
        ],
        "x",
    );
    let g = f.group_named("SECTOR_L1_L2").expect("内置分组存在");
    // symbols 升序为 ARB, ETH → 分别属于 L2(2) 与 L1(1)
    assert_close(g.at(0, 0), 2.0);
    assert_close(g.at(0, 1), 1.0);
    assert!(f.group_named("NOPE").is_err());
}

#[test]
fn group_scale_and_zscore_handle_singletons() {
    let close = sample_panel().factor("close").expect("close 列存在");
    let mut mapping = BTreeMap::new();
    mapping.insert("AAA".to_string(), 1.0);
    mapping.insert("BBB".to_string(), 1.0);
    mapping.insert("CCC".to_string(), 2.0);
    let groups = close.group_map(&mapping, None);
    let scaled = close.group_scale(&groups);
    // 组 1 极差正常 → 0 与 1；组 2 单一成员极差为 0 → 0.5
    assert_close(scaled.at(0, 0), 0.0);
    assert_close(scaled.at(0, 1), 1.0);
    assert_close(scaled.at(0, 2), 0.5);
    // 单一成员组的标准差无定义 → NaN
    assert!(close.group_zscore(&groups).at(0, 2).is_nan());
}

#[test]
fn group_normalize_scales_within_group() {
    let close = sample_panel().factor("close").expect("close 列存在");
    let mut mapping = BTreeMap::new();
    mapping.insert("AAA".to_string(), 1.0);
    mapping.insert("BBB".to_string(), 1.0);
    mapping.insert("CCC".to_string(), 2.0);
    let groups = close.group_map(&mapping, None);
    let out = close.group_normalize(&groups, 1.0);
    // 组 1：10 / 30 与 20 / 30
    assert_close(out.at(0, 0), 10.0 / 30.0);
    assert_close(out.at(0, 1), 20.0 / 30.0);
    // 组 2 只有自己 → 1.0
    assert_close(out.at(0, 2), 1.0);
}

#[test]
fn unmapped_symbols_yield_nan_group_result() {
    let close = sample_panel().factor("close").expect("close 列存在");
    let mut mapping = BTreeMap::new();
    mapping.insert("AAA".to_string(), 1.0);
    let groups = close.group_map(&mapping, None);
    let out = close.group_mean(&groups);
    assert_close(out.at(0, 0), 10.0);
    assert!(out.at(0, 1).is_nan());
    assert!(out.at(0, 2).is_nan());
}

#[test]
fn arithmetic_and_operators_agree() {
    let close = sample_panel().factor("close").expect("close 列存在");
    let open = sample_panel().factor("open").expect("open 列存在");
    let via_method = close.subtract(&open);
    let via_operator = &close - &open;
    for i in 0..via_method.values().len() {
        assert_close(via_method.values()[i], via_operator.values()[i]);
    }
    assert_close((&close * 2.0).at(0, 0), 20.0);
    assert_close((&close + 1.0).at(0, 0), 11.0);
    assert_close((-&close).at(0, 0), -10.0);
}

#[test]
fn divide_guards_against_near_zero() {
    let a = Factor::from_records(vec![("2024-01-01".into(), "AAA".into(), 1.0)], "a");
    let zero = Factor::from_records(vec![("2024-01-01".into(), "AAA".into(), 0.0)], "zero");
    assert!(a.divide(&zero).at(0, 0).is_nan());
    assert!(a.divide(0.0).at(0, 0).is_nan());
    assert_close(a.divide(2.0).at(0, 0), 0.5);
}

#[test]
fn unary_math_guards_domains() {
    let f = Factor::from_records(
        vec![
            ("2024-01-01".into(), "AAA".into(), -4.0),
            ("2024-01-01".into(), "BBB".into(), 0.0),
            ("2024-01-01".into(), "CCC".into(), 4.0),
        ],
        "x",
    );
    assert!(f.sqrt().at(0, 0).is_nan());
    assert_close(f.sqrt().at(0, 2), 2.0);
    assert!(f.ln().at(0, 0).is_nan());
    assert!(f.ln().at(0, 1).is_nan());
    assert!(f.inverse().at(0, 1).is_nan());
    assert_close(f.inverse().at(0, 2), 0.25);
    assert_close(f.sign().at(0, 0), -1.0);
    assert_close(f.sign().at(0, 1), 0.0);
    assert_close(f.abs().at(0, 0), 4.0);
}

#[test]
fn log_rejects_invalid_base() {
    let close = sample_panel().factor("close").expect("close 列存在");
    assert!(close.log(Some(1.0)).is_err());
    assert!(close.log(Some(-2.0)).is_err());
    let log2 = close.log(Some(2.0)).expect("底数合法");
    assert_close(log2.at(0, 0), 10f64.log2());
}

#[test]
fn signed_power_preserves_sign() {
    let f = Factor::from_records(
        vec![
            ("2024-01-01".into(), "AAA".into(), -4.0),
            ("2024-01-01".into(), "BBB".into(), 4.0),
        ],
        "x",
    );
    let sp = f.signed_power(0.5);
    assert_close(sp.at(0, 0), -2.0);
    assert_close(sp.at(0, 1), 2.0);
}

#[test]
fn maximum_minimum_propagate_nan() {
    let a = Factor::from_records(
        vec![
            ("2024-01-01".into(), "AAA".into(), 1.0),
            ("2024-01-01".into(), "BBB".into(), f64::NAN),
        ],
        "a",
    );
    assert_close(a.maximum(2.0).at(0, 0), 2.0);
    assert!(a.maximum(2.0).at(0, 1).is_nan());
    assert_close(a.minimum(2.0).at(0, 0), 1.0);
}

#[test]
fn where_selects_by_condition() {
    let close = sample_panel().factor("close").expect("close 列存在");
    let cond = close.gt(15.0);
    let out = where_(&cond, &close, 0.0);
    assert_close(out.at(0, 0), 0.0);
    assert_close(out.at(0, 1), 20.0);
    assert_close(out.at(0, 2), 30.0);
}

#[test]
fn comparisons_emit_zero_one() {
    let close = sample_panel().factor("close").expect("close 列存在");
    let gt = close.gt(15.0);
    assert_close(gt.at(0, 0), 0.0);
    assert_close(gt.at(0, 1), 1.0);
    assert_close(close.lt(15.0).at(0, 0), 1.0);
    assert_close(close.eq_val(10.0).at(0, 0), 1.0);
    assert_close(close.ne_val(10.0).at(0, 0), 0.0);
}

#[test]
fn alignment_intersects_index() {
    let a = Factor::from_records(
        vec![
            ("2024-01-01".into(), "AAA".into(), 1.0),
            ("2024-01-01".into(), "BBB".into(), 2.0),
            ("2024-01-02".into(), "AAA".into(), 3.0),
            ("2024-01-02".into(), "BBB".into(), 4.0),
        ],
        "a",
    );
    let b = Factor::from_records(
        vec![
            ("2024-01-02".into(), "AAA".into(), 10.0),
            ("2024-01-03".into(), "AAA".into(), 20.0),
        ],
        "b",
    );
    let sum = a.add(&b);
    // 交集仅 2024-01-02 × AAA
    assert_eq!(sum.timestamps(), ["2024-01-02"]);
    assert_eq!(sum.symbols(), ["AAA"]);
    assert_close(sum.at(0, 0), 13.0);
}

#[test]
fn ts_decay_linear_weights_oldest_most() {
    let f = Factor::from_records(
        vec![
            ("2024-01-01".into(), "AAA".into(), 0.0),
            ("2024-01-02".into(), "AAA".into(), 0.0),
            ("2024-01-03".into(), "AAA".into(), 3.0),
        ],
        "x",
    );
    // 权重按"由旧到新"为 3 : 2 : 1（Python 侧行为），当期值 3 → 3 × 1 / 6
    assert_close(f.ts_decay_linear(3, false).at(2, 0), 0.5);
    // dense = true 只对有效值加权，此处无缺失，结果一致
    assert_close(f.ts_decay_linear(3, true).at(2, 0), 0.5);
}

#[test]
fn ts_decay_exp_window_validates_factor() {
    let close = sample_panel().factor("close").expect("close 列存在");
    assert!(close.ts_decay_exp_window(3, 0.0, true).is_err());
    assert!(close.ts_decay_exp_window(3, 1.0, true).is_err());
    let d = close.ts_decay_exp_window(2, 0.5, true).expect("参数合法");
    // 权重 0.5 : 1（远 : 近）→ (10 × 0.5 + 11 × 1) / 1.5
    assert_close(d.at(1, 0), (10.0 * 0.5 + 11.0) / 1.5);
}

#[test]
fn ts_reversal_count_measures_direction_flips() {
    let f = Factor::from_records(
        vec![
            ("2024-01-01".into(), "AAA".into(), 1.0),
            ("2024-01-02".into(), "AAA".into(), 3.0),
            ("2024-01-03".into(), "AAA".into(), 2.0),
            ("2024-01-04".into(), "AAA".into(), 4.0),
        ],
        "x",
    );
    // 差分 +2, -1, +2 → 2 次符号变化 / 2 个相邻对
    assert_close(f.ts_reversal_count(4).at(3, 0), 1.0);
}

#[test]
fn ts_vr_and_cv_reject_bad_params() {
    let close = sample_panel().factor("close").expect("close 列存在");
    assert!(close.ts_vr(3, 0).is_err());
    assert!(close.ts_vr(3, 2).is_ok());
    // 恒定序列的变异系数为 0
    let flat = Factor::from_records(
        vec![
            ("2024-01-01".into(), "AAA".into(), 5.0),
            ("2024-01-02".into(), "AAA".into(), 5.0),
            ("2024-01-03".into(), "AAA".into(), 5.0),
        ],
        "x",
    );
    assert_close(flat.ts_cv(3).at(2, 0), 0.0);
}

#[test]
fn ts_autocorr_rejects_zero_lag() {
    let close = sample_panel().factor("close").expect("close 列存在");
    assert!(close.ts_autocorr(3, 0).is_err());
    assert!(close.ts_autocorr(3, 1).is_ok());
}

#[test]
fn ts_skewness_composes_rolling_moments() {
    let short = Factor::from_records(
        vec![
            ("2024-01-01".into(), "AAA".into(), 1.0),
            ("2024-01-02".into(), "AAA".into(), 2.0),
            ("2024-01-03".into(), "AAA".into(), 3.0),
        ],
        "x",
    );
    // 该算子由 ts_mean → ts_sum 复合而成，需 2 × window - 1 期才有首个有效值
    assert!(short.ts_skewness(3).at(2, 0).is_nan());

    let long = Factor::from_records(
        (1..=5)
            .map(|i| (format!("2024-01-0{i}"), "AAA".to_string(), i as f64))
            .collect(),
        "x",
    );
    // 偏离项恒为 1 → 分子 3 × 3、分母 3^1.5 × (3-1)(3-2)
    let expected = 9.0 / (3f64.powf(1.5) * 2.0);
    assert_close(long.ts_skewness(3).at(4, 0), expected);
}

#[test]
fn ts_kurtosis_needs_variation() {
    let flat = Factor::from_records(
        vec![
            ("2024-01-01".into(), "AAA".into(), 5.0),
            ("2024-01-02".into(), "AAA".into(), 5.0),
            ("2024-01-03".into(), "AAA".into(), 5.0),
        ],
        "x",
    );
    assert!(flat.ts_kurtosis(3).at(2, 0).is_nan());
    let varied = Factor::from_records(
        vec![
            ("2024-01-01".into(), "AAA".into(), 1.0),
            ("2024-01-02".into(), "AAA".into(), 2.0),
            ("2024-01-03".into(), "AAA".into(), 9.0),
        ],
        "x",
    );
    assert!(!varied.ts_kurtosis(3).at(2, 0).is_nan());
}

#[test]
fn ts_step_counts_periods() {
    let close = sample_panel().factor("close").expect("close 列存在");
    let s = close.ts_step(1);
    assert_close(s.at(0, 0), 1.0);
    assert_close(s.at(3, 2), 4.0);
}

#[test]
fn ts_scale_maps_into_unit_range() {
    let close = sample_panel().factor("close").expect("close 列存在");
    let s = close.ts_scale(3, 0.0);
    let v = s.at(2, 0);
    assert!((0.0..=1.0).contains(&v), "越界 {v}");
}

#[test]
fn readme_momentum_pipeline_runs() {
    let panel = sample_panel();
    let close = panel.factor("close").expect("close 列存在");
    let volume = panel.factor("volume").expect("volume 列存在");

    // 对应 README 的动量 + 成交量中性化组合
    let momentum = close.divide(&close.ts_delay(2)).subtract(1.0);
    let factor =
        vector_neut(&rank(&momentum), &rank(&volume.reverse())).rename("momentum_neut_volume");

    assert_eq!(factor.name, "momentum_neut_volume");
    assert_eq!(factor.n_periods(), 4);
    assert_eq!(factor.n_symbols(), 3);
    // 前两期动量不可得 → NaN；后两期应有有效值
    assert!(factor.row(0).iter().all(|v| v.is_nan()));
    assert!(factor.row(3).iter().any(|v| !v.is_nan()));
}

#[test]
fn quickstart_reversion_pipeline_runs() {
    let panel = sample_panel();
    let close = panel.factor("close").expect("close 列存在");
    let high = panel.factor("high").expect("high 列存在");
    let low = panel.factor("low").expect("low 列存在");
    let volume = panel.factor("volume").expect("volume 列存在");

    // 对应文档 quickstart 的反转因子
    let n = 3;
    let hi = high.ts_min(n);
    let relative_low = close.subtract(&hi).divide(&low.ts_max(n).subtract(&hi));
    let vol_deviation = volume.divide(&volume.ts_mean(n));
    let factor = relative_low
        .multiply(&vol_deviation.reverse().add(1.0).multiply(0.5).add(1.0))
        .rename("Reversion Alpha");

    assert_eq!(factor.name, "Reversion Alpha");
    assert_eq!(factor.values().len(), 12);
    assert!(factor.row(2).iter().any(|v| !v.is_nan()));
}

#[test]
fn to_records_roundtrips_through_csv() {
    let close = sample_panel().factor("close").expect("close 列存在");
    let csv = close.to_csv_string();
    assert!(csv.starts_with("timestamp,symbol,factor\n"));
    assert_eq!(csv.lines().count(), 13);
    assert_eq!(close.to_records().len(), 12);
}

#[test]
fn to_weights_reads_latest_period() {
    let close = sample_panel().factor("close").expect("close 列存在");
    let w = close.to_weights(None);
    assert_close(w["AAA"], 14.0);
    assert_close(w["CCC"], 35.0);
    let w0 = close.to_weights(Some("2024-01-01"));
    assert_close(w0["AAA"], 10.0);
}

#[test]
fn slice_helpers_narrow_the_panel() {
    let panel = sample_panel();
    let sliced = panel.slice_time(Some("2024-01-02"), Some("2024-01-03"));
    assert_eq!(sliced.timestamps().len(), 2);
    let narrowed = panel.slice_symbols(&["AAA", "CCC"]);
    assert_eq!(narrowed.symbols(), ["AAA", "CCC"]);
    assert_close(
        narrowed.factor("close").expect("close 列存在").at(0, 1),
        30.0,
    );
}

#[test]
fn select_narrows_columns() {
    let panel = sample_panel();
    let sub = panel.select(&["volume", "close"]).expect("两列都存在");
    // 与 slice_* 一样只收窄列，时间与标的索引不动
    assert_eq!(sub.column_names(), ["close", "volume"]);
    assert_eq!(sub.timestamps(), panel.timestamps());
    assert_eq!(sub.symbols(), panel.symbols());
    assert_close(sub.factor("close").expect("close").at(3, 2), 35.0);
    assert!(sub.factor("open").is_err(), "未选中的列应取不到");

    // 入参里的索引列被忽略（上游先滤掉再无条件加回），重复项只保留一次
    assert_eq!(
        panel
            .select(&["close", "timestamp", "symbol"])
            .unwrap()
            .column_names(),
        ["close"]
    );
    assert_eq!(
        panel.select(&["close", "close"]).unwrap().column_names(),
        ["close"]
    );

    // 列不存在 / 一列都不剩
    assert!(panel
        .select(&["vwap"])
        .expect_err("列不存在")
        .contains("vwap"));
    assert!(panel.select(&[]).is_err(), "空选择应报错");
    assert!(panel.select(&["timestamp"]).is_err(), "只给索引列应报错");
}

#[test]
fn to_csv_string_round_trips_and_blanks_nan() {
    let panel = Panel::from_csv_str(
        "\
timestamp,symbol,close,volume
2024-01-01,AAA,10,100
2024-01-01,BBB,20,
2024-01-02,AAA,,110
2024-01-02,BBB,19,190
",
    )
    .expect("可解析");

    let csv = panel.to_csv_string();
    // 表头 = timestamp,symbol + 升序列名；NaN 写成空字段（同 pandas to_csv）
    assert_eq!(
        csv,
        "\
timestamp,symbol,close,volume
2024-01-01,AAA,10,100
2024-01-01,BBB,20,
2024-01-02,AAA,,110
2024-01-02,BBB,19,190
"
    );

    // 读回后逐格一致
    let back = Panel::from_csv_str(&csv).expect("可读回");
    assert_eq!(back.column_names(), panel.column_names());
    assert_eq!(back.timestamps(), panel.timestamps());
    for name in panel.column_names() {
        let (a, b) = (panel.factor(name).unwrap(), back.factor(name).unwrap());
        for (x, y) in a.values().iter().zip(b.values()) {
            assert!(
                (x.is_nan() && y.is_nan()) || (x - y).abs() < 1e-12,
                "列 {name} 读回不一致：{x} vs {y}"
            );
        }
    }

    // to_records 的值顺序与 column_names 一致，可回喂 from_records
    let names = panel.column_names();
    let again = Panel::from_records(&names, panel.to_records()).expect("可重建");
    assert_eq!(again.to_csv_string(), csv);
}

#[test]
fn to_csv_writes_the_same_text_to_disk() {
    let panel = sample_panel();
    let path = std::env::temp_dir().join("phandas_rs_panel_to_csv.csv");
    panel.to_csv(&path).expect("写盘成功");
    let read_back = std::fs::read_to_string(&path).expect("可读回");
    assert_eq!(read_back, panel.to_csv_string());
    assert!(read_back.starts_with("timestamp,symbol,close,high,low,open,volume\n"));
    std::fs::remove_file(&path).expect("清理临时文件");
}

#[test]
fn show_and_info_render_without_panic() {
    let close = sample_panel().factor("close").expect("close 列存在");
    let rendered = close.show(2);
    assert!(rendered.contains("timestamp"));
    assert!(rendered.contains("AAA"));
    assert!(close.info().contains("Factor 'close'"));
    assert!(sample_panel().info().contains("Panel:"));
}

#[test]
fn free_functions_mirror_methods() {
    let close = sample_panel().factor("close").expect("close 列存在");
    assert_close(rank(&close).at(0, 0), close.rank().at(0, 0));
    assert_close(ts_mean(&close, 2).at(1, 0), close.ts_mean(2).at(1, 0));
    assert_close(ts_delay(&close, 1).at(1, 0), close.ts_delay(1).at(1, 0));
    assert_close(zscore(&close).at(0, 0), close.zscore().at(0, 0));
    assert_close(abs(&reverse(&close)).at(0, 0), 10.0);
}

#[test]
fn empty_intersection_yields_empty_factor() {
    let a = Factor::from_records(vec![("2024-01-01".into(), "AAA".into(), 1.0)], "a");
    let b = Factor::from_records(vec![("2024-02-01".into(), "BBB".into(), 2.0)], "b");
    let sum = a.add(&b);
    assert_eq!(sum.n_periods(), 0);
    assert_eq!(sum.values().len(), 0);
}

#[test]
fn scalar_on_the_left_mirrors_python_reflected_ops() {
    let f = Factor::from_records(
        vec![
            ("2024-01-01".into(), "AAA".into(), 2.0),
            ("2024-01-01".into(), "BBB".into(), 0.0),
            ("2024-01-01".into(), "CCC".into(), f64::NAN),
        ],
        "x",
    );
    // 可交换的两个直接转发到正向方法，因子名与 Python 的 __radd__ / __rmul__ 一致
    assert_close((2.0 + &f).at(0, 0), 4.0);
    assert_eq!((2.0 + &f).name, "(x+2)");
    assert_close((3.0 * &f).at(0, 0), 6.0);
    assert_eq!((3.0 * &f).name, "(x*3)");
    // 不可交换的两个必须调换操作数，且因子名把标量写在左边
    assert_close((10.0 - &f).at(0, 0), 8.0);
    assert_close((&f - 10.0).at(0, 0), -8.0);
    assert_eq!((10.0 - &f).name, "(10-x)");
    assert_close((10.0 / &f).at(0, 0), 5.0);
    assert_eq!((10.0 / &f).name, "(10/x)");
    // 除数恰为 0 → NaN；NaN 位置照常传播
    assert!((10.0 / &f).at(0, 1).is_nan());
    assert!((10.0 - &f).at(0, 2).is_nan());
    // 右值也可以是拥有所有权的因子
    assert_close((10.0 - f.clone()).at(0, 0), 8.0);
    assert_close((10.0 / f.clone()).at(0, 0), 5.0);
    // Rust 没有 `**` 运算符，标量为底的幂只有方法形式
    assert_close(f.scalar_power(3.0).at(0, 0), 9.0);
    assert_close(f.scalar_power(3.0).at(0, 1), 1.0);
    assert_eq!(f.scalar_power(3.0).name, "(3**x)");
}

#[test]
fn scalar_div_guards_on_exact_zero_only() {
    let tiny = Factor::from_records(vec![("2024-01-01".into(), "AAA".into(), 1e-12)], "tiny");
    let one = Factor::from_records(vec![("2024-01-01".into(), "AAA".into(), 1.0)], "one");
    // 复刻 Python __rtruediv__：判据是精确等零，1e-12 会照除出一个极大值
    assert!((1.0 / &tiny).at(0, 0) > 1e11);
    // 而因子除因子走 |y| > 1e-10 判据，同一位置得 NaN
    assert!(one.divide(&tiny).at(0, 0).is_nan());
}
