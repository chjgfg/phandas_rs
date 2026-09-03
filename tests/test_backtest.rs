//! `backtest` 模块的集成测试。参考值全部取自上游依赖的 pandas / scipy 实测输出。

use phandas_rs::backtest::date::{parse_days, shift_days, span_days};
use phandas_rs::backtest::stats::{cummax, kurtosis_pearson, pearson_r, quantile, skew};
use phandas_rs::backtest::*;
use phandas_rs::factor::Factor;
use phandas_rs::factor::numeric::norm_cdf;

fn assert_close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-9, "期望 {b} 实际 {a}");
}

/// 参考样本：10 期收益率，pandas / scipy 的各项统计量已实测记录在测试断言里。
fn sample_returns() -> Vec<(String, f64)> {
    let rs = [0.01, -0.02, 0.03, -0.005, 0.012, -0.008, 0.02, -0.015, 0.005, 0.011];
    (0..10)
        .map(|i| (format!("2024-01-{:02}", i + 1), rs[i]))
        .collect()
}

#[test]
fn date_parsing_and_arithmetic() {
    assert_eq!(parse_days("1970-01-01"), Some(0));
    assert_eq!(parse_days("1970-01-02"), Some(1));
    assert_eq!(parse_days("1969-12-31"), Some(-1));
    // 带时间部分只取日期，空格与 T 两种分隔都认
    assert_eq!(parse_days("1970-01-02 08:30:00"), Some(1));
    assert_eq!(parse_days("1970-01-02T08:30:00"), Some(1));
    // 闰年与非闰年
    assert_eq!(parse_days("2024-02-29").map(|_| true), Some(true));
    assert_eq!(parse_days("2023-02-29"), None);
    assert_eq!(parse_days("1900-02-29"), None);
    assert_eq!(parse_days("2000-02-29").map(|_| true), Some(true));
    // 非法格式
    assert_eq!(parse_days("2024-13-01"), None);
    assert_eq!(parse_days("2024-01-32"), None);
    assert_eq!(parse_days("not-a-date"), None);
    assert_eq!(parse_days("2024-01"), None);
}

#[test]
fn date_shift_crosses_month_and_year() {
    assert_eq!(shift_days("2024-01-01", -1).as_deref(), Some("2023-12-31"));
    assert_eq!(shift_days("2024-03-01", -1).as_deref(), Some("2024-02-29"));
    assert_eq!(shift_days("2023-03-01", -1).as_deref(), Some("2023-02-28"));
    assert_eq!(shift_days("2024-12-31", 1).as_deref(), Some("2025-01-01"));
    // 时间部分原样保留
    assert_eq!(
        shift_days("2024-01-01 08:00:00", -1).as_deref(),
        Some("2023-12-31 08:00:00")
    );
    assert_eq!(shift_days("bad", -1), None);
}

#[test]
fn date_span_counts_calendar_days() {
    assert_eq!(span_days("2024-01-01", "2024-01-10"), Some(9));
    assert_eq!(span_days("2024-01-10", "2024-01-01"), Some(-9));
    // 2024 是闰年，1 月 1 日到次年 1 月 1 日跨 366 天
    assert_eq!(span_days("2024-01-01", "2025-01-01"), Some(366));
    assert_eq!(span_days("2023-01-01", "2024-01-01"), Some(365));
    assert_eq!(span_days("2024-01-01", "bad"), None);
}

#[test]
fn quantile_matches_pandas_linear_interpolation() {
    let rs: Vec<f64> = sample_returns().iter().map(|(_, r)| *r).collect();
    assert_close(quantile(&rs, 0.05), -0.017_75);
    assert_close(quantile(&rs, 0.25), -0.007_25);
    assert_close(quantile(&rs, 0.5), 0.007_5);
    // 单点与全 NaN
    assert_close(quantile(&[3.0], 0.5), 3.0);
    assert!(quantile(&[f64::NAN], 0.5).is_nan());
    assert!(quantile(&[], 0.5).is_nan());
}

#[test]
// 参考值照抄 scipy / pandas 的 17 位输出，便于逐位比对
#[allow(clippy::excessive_precision)]
fn skew_and_kurtosis_match_scipy_biased() {
    let rs: Vec<f64> = sample_returns().iter().map(|(_, r)| *r).collect();
    assert_close(skew(&rs), -0.011_780_423_599_291_627);
    // scipy 的 kurtosis(fisher=False)，未减 3
    assert_close(kurtosis_pearson(&rs), 2.025_007_864_108_210_3);
    // 取值全同时 scipy 给 NaN，此处一致
    assert!(skew(&[5.0, 5.0, 5.0]).is_nan());
    assert!(kurtosis_pearson(&[5.0, 5.0, 5.0]).is_nan());
}

#[test]
fn pearson_r_and_cummax() {
    assert_close(pearson_r(&[1.0, 2.0, 3.0], &[2.0, 4.0, 6.0]), 1.0);
    assert_close(pearson_r(&[1.0, 2.0, 3.0], &[6.0, 4.0, 2.0]), -1.0);
    assert!(pearson_r(&[1.0, 2.0], &[3.0, 3.0]).is_nan());
    assert!(pearson_r(&[1.0], &[1.0]).is_nan());
    assert_eq!(cummax(&[1.0, 3.0, 2.0, 5.0, 4.0]), vec![1.0, 3.0, 3.0, 5.0, 5.0]);
}

#[test]
// 参考值照抄 scipy / pandas 的 17 位输出，便于逐位比对
#[allow(clippy::excessive_precision)]
fn norm_cdf_matches_scipy() {
    assert_close(norm_cdf(0.0), 0.5);
    assert_close(norm_cdf(1.0), 0.841_344_746_068_542_93);
    assert_close(norm_cdf(-1.0), 0.158_655_253_931_457_07);
    assert_close(norm_cdf(1.959_963_984_540_054), 0.975);
    assert_close(norm_cdf(-1.959_963_984_540_054), 0.025);
    assert_close(norm_cdf(2.5), 0.993_790_334_674_223_84);
    // 远尾走连分式分支
    assert_close(norm_cdf(7.5), 0.999_999_999_999_968_14);
    assert_close(norm_cdf(-7.5), 3.190_891_672_910_884_4e-14);
    assert_close(norm_cdf(40.0), 1.0);
    assert_close(norm_cdf(-40.0), 0.0);
    assert!(norm_cdf(f64::NAN).is_nan());
}
#[test]
// 参考值照抄 scipy / pandas 的 17 位输出，便于逐位比对
#[allow(clippy::excessive_precision)]
fn performance_metrics_match_pandas_scipy_reference() {
    let (m, periods) = performance_metrics(&sample_returns(), 0.03).expect("样本足够");
    assert_close(m.total_return, 0.039_569_571_740_008_636);
    assert_close(m.annual_return, 3.825_090_418_262_354);
    assert_close(m.annual_volatility, 0.301_673_112_269_997_71);
    assert_close(m.sharpe_ratio, 12.580_141_430_919_918);
    assert_close(m.sortino_ratio, 29.288_476_353_314_685);
    assert_close(m.calmar_ratio, 191.254_520_913_117_52);
    assert_close(m.max_drawdown, -0.02);
    assert_close(m.linearity, 0.611_664_598_467_336_45);
    assert_close(m.var_95, -0.017_75);
    assert_close(m.cvar, -0.02);
    assert_close(m.psr, 0.618_485_735_232_395_14);
    assert_eq!(periods.len(), 4);
}

#[test]
fn performance_metrics_needs_two_returns() {
    assert!(performance_metrics(&[], 0.03).is_none());
    assert!(performance_metrics(&[("2024-01-01".into(), 0.01)], 0.03).is_none());
}

#[test]
fn psr_is_zero_for_degenerate_samples() {
    assert_close(psr(&[0.01], 0.0), 0.0);
    assert_close(psr(&[], 0.0), 0.0);
    // 恒定收益率：标准差为 0 → 观测夏普取 0，偏度峰度为 NaN → 调整项 NaN → 结果 NaN
    assert!(psr(&[0.01, 0.01, 0.01], 0.0).is_nan());
}

#[test]
fn drawdown_periods_are_sorted_deepest_first() {
    let dates: Vec<String> = (1..=10).map(|i| format!("2024-01-{i:02}")).collect();
    let equity: Vec<f64> = {
        let mut acc = 1.0;
        [0.01, -0.02, 0.03, -0.005, 0.012, -0.008, 0.02, -0.015, 0.005, 0.011]
            .iter()
            .map(|r| {
                acc *= 1.0 + r;
                acc
            })
            .collect()
    };
    let periods = identify_drawdown_periods(&dates, &equity);
    assert_eq!(periods.len(), 4);
    // 最深一段：第 2 期跌 2%，第 3 期即回到新高
    assert_eq!(periods[0].start, "2024-01-02");
    assert_eq!(periods[0].end, "2024-01-03");
    assert_close(periods[0].depth, -0.02);
    assert_eq!(periods[0].duration_days, 1);
    // 次深一段跨两天才回本
    assert_eq!(periods[1].start, "2024-01-08");
    assert_eq!(periods[1].end, "2024-01-10");
    assert_close(periods[1].depth, -0.015);
    assert_eq!(periods[1].duration_days, 2);
    // 深度严格递增（越往后越浅）
    assert!(periods.windows(2).all(|w| w[0].depth <= w[1].depth));
}

#[test]
fn drawdown_periods_handles_unrecovered_tail() {
    let dates: Vec<String> = (1..=3).map(|i| format!("2024-01-{i:02}")).collect();
    // 一路下跌，扫描结束时仍在回撤中 → 以末期收尾
    let periods = identify_drawdown_periods(&dates, &[1.0, 0.9, 0.8]);
    assert_eq!(periods.len(), 1);
    assert_eq!(periods[0].start, "2024-01-02");
    assert_eq!(periods[0].end, "2024-01-03");
    assert_close(periods[0].depth, -0.2);
    // 单调上涨则没有回撤段
    assert!(identify_drawdown_periods(&dates, &[1.0, 1.1, 1.2]).is_empty());
}

#[test]
fn portfolio_tracks_cash_positions_and_costs() {
    let mut p = Portfolio::new(1000.0);
    let prices: std::collections::BTreeMap<String, f64> =
        [("AAA".to_string(), 10.0)].into_iter().collect();
    // 买 10 股 @10，手续费 1% → 现金 1000 - 100 - 1
    p.execute_trade("AAA", 10.0, 10.0, (0.01, 0.02), "2024-01-01");
    assert_close(p.cash(), 899.0);
    assert_close(*p.positions().get("AAA").expect("有持仓"), 10.0);
    p.revalue(&prices);
    assert_close(p.total_value(), 999.0);
    assert_close(*p.holdings().get("AAA").expect("有市值"), 100.0);
    // 全部卖出走卖出费率 2% → 现金 899 + 100 - 2，持仓被移除
    p.execute_trade("AAA", -10.0, 10.0, (0.01, 0.02), "2024-01-02");
    assert_close(p.cash(), 997.0);
    assert!(p.positions().is_empty());
    assert_eq!(p.trade_log().len(), 2);
    assert_close(p.trade_log()[1].cost, 2.0);
    // 净值历史由调用方显式记录，一天一条
    p.record("2024-01-02");
    assert_eq!(p.history().len(), 1);
}
/// 成交价因子：AAA 与 CCC 不动，BBB 逐期上涨。
fn price_factor() -> Factor {
    let mut recs = Vec::new();
    for (i, ts) in ["2024-01-01", "2024-01-02", "2024-01-03", "2024-01-04"].iter().enumerate() {
        recs.push((ts.to_string(), "AAA".to_string(), 10.0));
        recs.push((ts.to_string(), "BBB".to_string(), 20.0 + 2.0 * i as f64));
        recs.push((ts.to_string(), "CCC".to_string(), 30.0));
    }
    Factor::from_records(recs, "open")
}

/// 策略因子：每期都是标准的美元中性信号（多头 +0.5、空头 -0.5、合计 0）。
fn signal_factor() -> Factor {
    let mut recs = Vec::new();
    for ts in ["2024-01-01", "2024-01-02", "2024-01-03", "2024-01-04"] {
        recs.push((ts.to_string(), "AAA".to_string(), -0.5));
        recs.push((ts.to_string(), "BBB".to_string(), 0.5));
        recs.push((ts.to_string(), "CCC".to_string(), 0.0));
    }
    Factor::from_records(recs, "alpha")
}

#[test]
fn is_signal_gates_the_as_is_path() {
    // 这是 target_holdings 判断"因子能否直接当权重"的闸门
    assert!(signal_factor().is_signal(None));
    assert!(signal_factor().is_signal(Some("2024-01-01")));
    assert!(!signal_factor().is_signal(Some("2024-02-01")));
    let raw = Factor::from_records(
        vec![
            ("2024-01-01".into(), "AAA".into(), -3.0),
            ("2024-01-01".into(), "BBB".into(), 3.0),
        ],
        "raw",
    );
    assert!(!raw.is_signal(None));
}

#[test]
fn backtest_runs_the_event_loop() {
    let bt = Backtester::new(&price_factor(), &signal_factor())
        .transaction_cost(0.0, 0.0)
        .run()
        .expect("数据足够")
        .calculate_metrics(0.03);

    // 播种一条 + 三个交易日；首个交易日是 01-02（01-01 没有前一期故不交易），
    // 播种日期取其前一天，正好落回 01-01
    let equity = bt.equity();
    assert_eq!(equity.len(), 4);
    assert_eq!(equity[0].0, "2024-01-01");
    assert_close(equity[0].1, 1000.0);
    // 首个交易日建仓前净值仍是初始资金
    assert_eq!(equity[1].0, "2024-01-02");
    assert_close(equity[1].1, 1000.0);
    // 次日：空 AAA 50 股不赚不亏，多 BBB 500/22 股涨到 24
    assert_close(equity[2].1, 1000.0 + 500.0 / 22.0 * 24.0 - 500.0);

    // 首日只对 AAA / BBB 下单，CCC 目标为 0 且无持仓故不成交
    let trades = bt.trades();
    assert_eq!(trades.iter().filter(|t| t.date == "2024-01-02").count(), 2);
    assert_close(trades[0].trade_value, -500.0);
    assert_close(trades[1].trade_value, 500.0);
    assert_close(trades[0].cost, 0.0);

    assert!(bt.metrics().is_some());
    assert_eq!(bt.turnover().len(), 3);
    assert_eq!(bt.drawdown().len(), 4);
    assert!(bt.skipped_dates().is_empty());
    assert!(bt.summary().contains("strategy='alpha'"));
    assert!(format!("{bt}").contains("days=4"));
}

#[test]
fn full_rebalance_still_records_one_equity_point_per_day() {
    let bt = Backtester::new(&price_factor(), &signal_factor())
        .full_rebalance(true)
        .run()
        .expect("数据足够")
        .calculate_metrics(0.03);
    // 与上游的关键差异：清仓与建仓算同一天的一次调仓，不写第二条净值
    assert_eq!(bt.equity().len(), 4);
    // 因此换手率在该模式下也算得出来
    assert!(!bt.turnover().is_empty());
    assert!(bt.metrics().is_some());
}

#[test]
fn zero_factor_generates_no_trades() {
    let mut recs = Vec::new();
    for ts in ["2024-01-01", "2024-01-02", "2024-01-03", "2024-01-04"] {
        for s in ["AAA", "BBB", "CCC"] {
            recs.push((ts.to_string(), s.to_string(), 0.0));
        }
    }
    let flat = Factor::from_records(recs, "flat");
    let bt = Backtester::new(&price_factor(), &flat)
        .neutralization(Neutralization::None)
        .run()
        .expect("数据足够")
        .calculate_metrics(0.03);
    assert!(bt.trades().is_empty());
    assert!(bt.turnover().is_empty());
    // 净值一路持平
    assert!(bt.equity().iter().all(|(_, v)| (v - 1000.0).abs() < 1e-9));
}

#[test]
fn nan_periods_are_dropped_whole() {
    let mut recs = Vec::new();
    for (i, ts) in ["2024-01-01", "2024-01-02", "2024-01-03", "2024-01-04"].iter().enumerate() {
        recs.push((ts.to_string(), "AAA".to_string(), -0.5));
        // 第三期 BBB 缺失 → 整期被丢弃
        recs.push((ts.to_string(), "BBB".to_string(), if i == 2 { f64::NAN } else { 0.5 }));
        recs.push((ts.to_string(), "CCC".to_string(), 0.0));
    }
    let holey = Factor::from_records(recs, "holey");
    let bt = Backtester::new(&price_factor(), &holey)
        .transaction_cost(0.0, 0.0)
        .run()
        .expect("数据足够")
        .calculate_metrics(0.03);
    assert_eq!(bt.skipped_dates(), ["2024-01-03"]);
    // 前一期策略缺失时目标为空 → 当期清空全部持仓，故 01-04 有卖出成交
    assert!(bt.trades().iter().any(|t| t.date == "2024-01-04"));
    assert!(bt.portfolio().positions().is_empty());
}

#[test]
fn run_rejects_insufficient_data() {
    let one = Factor::from_records(vec![("2024-01-01".into(), "AAA".into(), 1.0)], "one");
    let err = Backtester::new(&one, &one).run().expect_err("只有一期");
    assert!(err.contains("Insufficient overlapping dates"));

    // 两期有交集，但策略因子第一期全 NaN 导致找不到起始点
    let px = Factor::from_records(
        vec![
            ("2024-01-01".into(), "AAA".into(), 10.0),
            ("2024-01-02".into(), "AAA".into(), 10.0),
        ],
        "open",
    );
    let bad = Factor::from_records(
        vec![
            ("2024-01-01".into(), "AAA".into(), f64::NAN),
            ("2024-01-02".into(), "AAA".into(), 1.0),
        ],
        "bad",
    );
    let err = Backtester::new(&px, &bad).run().expect_err("无有效起始点");
    assert!(err.contains("No valid start date"));
}

#[test]
fn neutralization_parse_rejects_unknown() {
    assert_eq!(Neutralization::parse("none"), Ok(Neutralization::None));
    assert_eq!(Neutralization::parse("MARKET"), Ok(Neutralization::Market));
    assert!(Neutralization::parse("mkt").is_err());
    assert_eq!(Neutralization::default(), Neutralization::Market);
}

#[test]
fn benchmark_is_equal_weight_buy_and_hold() {
    let bt = Backtester::new(&price_factor(), &signal_factor())
        .run()
        .expect("数据足够");
    let bench = bt.benchmark_equity();
    // 从首个交易日 01-02 起算，共 3 期
    assert_eq!(bench.len(), 3);
    assert_eq!(bench[0].0, "2024-01-02");
    assert_close(bench[0].1, 1000.0);
    // 三分之一资金买 BBB，价格从 22 涨到 26
    assert_close(bench[2].1, 1000.0 / 3.0 * 2.0 + 1000.0 / 3.0 / 22.0 * 26.0);
}

#[test]
fn reports_render_without_metrics() {
    let bt = Backtester::new(&price_factor(), &signal_factor());
    assert_eq!(bt.summary(), "Backtester(no metrics available)");
    assert_eq!(bt.drawdown_report(5), "Drawdown Periods: none detected");
    assert!(format!("{bt}").contains("entry_price=open"));
}
