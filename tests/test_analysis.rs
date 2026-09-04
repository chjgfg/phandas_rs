//! `analysis` 模块的单元 / 集成测试。
//!
//! 数值断言钉在**上游 Python `analysis.py` 在 `tests/data/panel.csv` 上的真值**上：
//! 价格 = `close`、因子 = `[signal(rank(close)), rank(close), close, volume]`、持有期 `[1, 3]`，
//! 由 `.omc/difftest/run_py_analysis.py` 生成、`compare_analysis.py` 逐格比对过
//! （容差 1e-9，272 项全一致）。`summary()` 与相关矩阵文本另与上游 `summary()` 逐字符比对过。
//!
//! 因子表里前两个的 NaN 是整期的、后两个只缺单格，两类都要有：`stats()` 的 turnover 是
//! "每标的先取均值、再对标的取均值"的两级平均，只在各标的有效差分数不齐时才和池化平均
//! 分得开，而整期 NaN 恰好让两者相等。

use phandas_rs::analysis::{analyze, corr, CorrMethod, FactorAnalyzer, IcMethod, DEFAULT_HORIZONS};
use phandas_rs::factor::{Factor, Panel};

const PANEL: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/panel.csv");

/// 绝对 + 相对容差断言。
fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-12 + 1e-9 * a.abs().max(b.abs())
}

/// 从行主序矩阵构造因子；`rows[ti][si]`，NaN 直接写进矩阵。
fn factor_of(name: &str, ts: &[&str], symbols: &[&str], rows: &[Vec<f64>]) -> Factor {
    let mut records = Vec::new();
    for (ti, t) in ts.iter().enumerate() {
        for (si, s) in symbols.iter().enumerate() {
            records.push((t.to_string(), s.to_string(), rows[ti][si]));
        }
    }
    Factor::from_records(records, name)
}

/// 在面板上构造上游真值所用的分析器（价格 = `close`，因子 = `[signal(rank(close)), rank(close))]`，
/// 持有期 `[1, 3]`）。测试需要把因子与价格提升到 `'static`，故用 `Box::leak`（测试泄漏无害）。
fn panel_analyzer() -> FactorAnalyzer<'static> {
    let price = Box::leak(Box::new(
        Panel::from_csv(PANEL)
            .expect("panel.csv 可解析")
            .factor("close")
            .expect("close"),
    ));
    let alpha1 = Box::leak(Box::new(price.rank().signal()));
    let alpha2 = Box::leak(Box::new(price.rank()));
    let factors: Vec<&'static Factor> = vec![alpha1, alpha2];
    FactorAnalyzer::new(&factors, price, Some(&[1, 3])).expect("非空因子表")
}

// ---------------------------------------------------------------------------
// 构造与访问器
// ---------------------------------------------------------------------------

#[test]
fn analyze_creates_analyzer_with_default_horizons() {
    let fa = panel_analyzer();
    assert_eq!(fa.factors().len(), 2);
    assert_eq!(fa.horizons(), &[1, 3]);
    assert_eq!(DEFAULT_HORIZONS, [1, 7, 30]);
}

#[test]
fn empty_factors_raise_error() {
    let panel = Panel::from_csv(PANEL).unwrap();
    let close = panel.factor("close").unwrap();
    match analyze(&[], &close, None) {
        Err(msg) => assert!(msg.contains("at least one factor")),
        Ok(_) => panic!("空因子表应返回 Err"),
    }
}

#[test]
fn display_matches_upstream_repr() {
    let fa = panel_analyzer();
    assert_eq!(
        fa.to_string(),
        "FactorAnalyzer(factors=['signal(rank(close))', 'rank(close)'], horizons=[1, 3])"
    );
}

// ---------------------------------------------------------------------------
// IC：逐格对齐上游真值
// ---------------------------------------------------------------------------

#[test]
fn ic_spearman_matches_upstream_panel() {
    let fa = panel_analyzer();
    let ics = fa.ic(IcMethod::Spearman);

    // 两个因子分别是 signal 与 rank，Spearman IC 口径下与上游一致
    for fic in &ics {
        let s1 = fic.for_horizon(1).expect("h=1");
        assert!(approx(s1.ic_mean, 0.342_348_758_898_243_3), "ic_mean h1");
        assert!(approx(s1.ic_std, 0.644_220_109_514_64), "ic_std h1");
        assert!(approx(s1.ir, 0.531_415_821_769_629_8), "ir h1");
        assert!(approx(s1.t_stat, 1.594_247_465_308_889_2), "t_stat h1");
        assert_eq!(s1.ic_series.len(), 9, "h1 有效期数");
        // 首期
        let (ts0, ic0) = &s1.ic_series[0];
        assert_eq!(ts0, "2024-01-01");
        assert!(approx(*ic0, 0.5), "首期 IC");

        let s3 = fic.for_horizon(3).expect("h=3");
        assert!(approx(s3.ic_mean, 0.200_125_400_185_665_38), "ic_mean h3");
        assert!(approx(s3.ic_std, 0.546_951_710_306_199_3), "ic_std h3");
        assert_eq!(s3.ic_series.len(), 7, "h3 有效期数");
    }
}

#[test]
fn ic_pearson_matches_upstream_panel() {
    // 注意：上游 pearson 与 spearman 同面板下不同（因子直接取原值），
    // n 从 9 降到 8（2024-01-02 被有效性闸门剔除）。
    let fa = panel_analyzer();
    let ics = fa.ic(IcMethod::Pearson);
    for fic in &ics {
        let s1 = fic.for_horizon(1).expect("h=1");
        assert!(approx(s1.ic_mean, 0.389_792_911_533_100_5), "ic_mean h1");
        assert!(approx(s1.ic_std, 0.595_132_995_936_656_4), "ic_std h1");
        assert_eq!(s1.ic_series.len(), 8, "h1 pearson 有效期数");
        let (ts0, ic0) = &s1.ic_series[0];
        assert_eq!(ts0, "2024-01-01");
        assert!(approx(*ic0, 0.829_738_483_663_489_8), "首期 pearson IC");
    }
}

#[test]
fn ic_series_is_empty_when_no_overlap() {
    // 两个标的、一期样本，横截面不足 3 个有效对 → 空序列 → 全 NaN 统计
    let ts = vec!["2024-01-01", "2024-01-02", "2024-01-03"];
    let syms = vec!["A", "B", "C"];
    let f = factor_of(
        "f",
        &ts,
        &syms,
        &[
            vec![1.0, 2.0, 3.0],
            vec![2.0, 3.0, 4.0],
            vec![3.0, 4.0, 5.0],
        ],
    );
    let p = factor_of(
        "p",
        &ts,
        &syms,
        &[
            vec![1.0, 2.0, 4.0],
            vec![2.0, 4.0, 8.0],
            vec![4.0, 8.0, 16.0],
        ],
    );
    let fa = FactorAnalyzer::new(&[&f], &p, Some(&[1])).expect("ok");
    let ics = fa.ic(IcMethod::Spearman);
    assert_eq!(ics[0].by_horizon.len(), 1);
    let s = &ics[0].by_horizon[0];
    assert!(s.ic_mean.is_nan() && s.ic_std.is_nan());
    assert!(s.ic_series.is_empty());
}

// ---------------------------------------------------------------------------
// stats
// ---------------------------------------------------------------------------

#[test]
fn stats_matches_upstream_panel() {
    let fa = panel_analyzer();
    let statss = fa.stats();
    for fs in &statss {
        assert!(approx(fs.coverage, 40.0 / 48.0), "coverage");
        assert!(approx(fs.turnover, 0.053_571_428_571_428_57), "turnover");
        assert!(fs.autocorr.is_nan(), "autocorr（上游同面板也是 NaN）");
    }
    // 覆盖率的几何含义：rank/signal 让含 NaN 的 2024-01-04 整期失效（4 格）+ 2×2 边界 = 40/48
    assert!(approx(statss[0].coverage, 40.0 / 48.0));
}

#[test]
fn stats_turnover_is_two_level_mean_not_pooled() {
    // close / volume 各只缺一格（(01-04, AAA) 与 (01-02, CCC)），故各标的的有效差分个数
    // 不齐——这时上游 rank_diff.mean().mean() 的两级平均与"把全部格子池化求一个均值"
    // 分得开：池化会给 0.14682539682539683，上游给 0.14267676767676768。
    let price = Box::leak(Box::new(
        Panel::from_csv(PANEL).unwrap().factor("close").unwrap(),
    ));
    let volume = Box::leak(Box::new(
        Panel::from_csv(PANEL).unwrap().factor("volume").unwrap(),
    ));
    let factors: Vec<&'static Factor> = vec![price, volume];
    let fa = FactorAnalyzer::new(&factors, price, Some(&[1])).unwrap();
    let statss = fa.stats();

    assert!(approx(statss[0].coverage, 47.0 / 48.0), "close coverage");
    assert!(
        approx(statss[0].turnover, 0.142_676_767_676_767_68),
        "close turnover 应为两级平均，得 {}",
        statss[0].turnover
    );
    assert!(
        approx(statss[0].autocorr, 0.731_883_257_172_468_6),
        "close autocorr"
    );
    assert!(
        approx(statss[1].turnover, 0.049_242_424_242_424_24),
        "volume turnover，得 {}",
        statss[1].turnover
    );
    assert!(
        approx(statss[1].autocorr, 0.885_325_777_150_328_2),
        "volume autocorr"
    );
}

#[test]
fn stats_turnover_is_nan_when_undecidable() {
    // 上游只在 rank_diff 为空（0 期或 0 标的）时给 0；只有 1 期或差分整块全 NaN 时是 NaN
    let one = factor_of("one", &["2024-01-01"], &["A", "B"], &[vec![1.0, 2.0]]);
    let fa = FactorAnalyzer::new(&[&one], &one, Some(&[1])).unwrap();
    assert!(fa.stats()[0].turnover.is_nan(), "单期 turnover 应为 NaN");

    let half = factor_of(
        "half",
        &["2024-01-01", "2024-01-02"],
        &["A", "B"],
        &[vec![1.0, 2.0], vec![f64::NAN, f64::NAN]],
    );
    let fa = FactorAnalyzer::new(&[&half], &half, Some(&[1])).unwrap();
    assert!(
        fa.stats()[0].turnover.is_nan(),
        "差分全 NaN 时 turnover 应为 NaN"
    );

    // 0 期：上游 rank_diff.empty 为真 → 0.0
    let empty = Factor::new(vec![], vec!["A".to_string()], vec![], "empty").unwrap();
    let fa = FactorAnalyzer::new(&[&empty], &empty, Some(&[1])).unwrap();
    let s = &fa.stats()[0];
    assert_eq!(s.turnover, 0.0, "0 期 turnover 应为 0.0");
    assert_eq!(s.coverage, 0.0);
}

#[test]
fn empty_horizons_falls_back_to_default() {
    // 上游 `horizons or _DEFAULT_HORIZONS`：空 list 是 falsy，回落到 [1, 7, 30]
    let panel = Panel::from_csv(PANEL).unwrap();
    let close = panel.factor("close").unwrap();
    let f = close.rank().signal();
    assert_eq!(
        FactorAnalyzer::new(&[&f], &close, Some(&[]))
            .unwrap()
            .horizons(),
        &DEFAULT_HORIZONS
    );
    assert_eq!(
        analyze(&[&f], &close, Some(&[])).unwrap().horizons(),
        &DEFAULT_HORIZONS
    );
    // 非空时原样透传
    assert_eq!(
        FactorAnalyzer::new(&[&f], &close, Some(&[5]))
            .unwrap()
            .horizons(),
        &[5]
    );
}

// ---------------------------------------------------------------------------
// correlation
// ---------------------------------------------------------------------------

#[test]
fn correlation_matches_upstream_panel() {
    let fa = panel_analyzer();
    for method in [CorrMethod::Pearson, CorrMethod::Spearman] {
        let cm = fa.correlation(method);
        assert_eq!(cm.len(), 2);
        assert_eq!(
            cm.names(),
            &["signal(rank(close))".to_string(), "rank(close)".to_string()]
        );
        assert_eq!(cm.n_obs(), 40);
        assert!(approx(cm.at(0, 0), 1.0) && approx(cm.at(1, 1), 1.0));
        assert!(approx(
            cm.get("signal(rank(close))", "rank(close)").unwrap(),
            1.0
        ));
        assert!(cm.at(0, 0).is_finite());
    }
}

#[test]
fn single_factor_correlation_is_empty() {
    let panel = Panel::from_csv(PANEL).unwrap();
    let close = panel.factor("close").unwrap();
    let f = close.rank().signal();
    let fa = FactorAnalyzer::new(&[&f], &close, None).unwrap();
    let cm = fa.correlation(CorrMethod::Pearson);
    assert!(cm.is_empty());
    assert_eq!(cm.len(), 0);
}

#[test]
fn correlation_empty_when_disjoint_grids() {
    let ts_a = vec!["2024-01-01", "2024-01-02"];
    let ts_b = vec!["2024-02-01", "2024-02-02"];
    let syms = vec!["A"];
    let x = factor_of("x", &ts_a, &syms, &[vec![1.0], vec![2.0]]);
    let y = factor_of("y", &ts_b, &syms, &[vec![1.0], vec![2.0]]);
    let fa = FactorAnalyzer::new(&[&x, &y], &x, None).unwrap();
    let cm = fa.correlation(CorrMethod::Pearson);
    assert!(cm.is_empty());
}

#[test]
fn correlation_matrix_table_matches_pandas_layout() {
    let fa = panel_analyzer();
    let cm = fa.correlation(CorrMethod::Pearson);
    // 由 pandas 对同一相关矩阵 `to_string(float_format=lambda x: f'{x:.4f}')` 逐字符比对过
    let expected = concat!(
        "                     signal(rank(close))  rank(close)\n",
        "signal(rank(close))               1.0000       1.0000\n",
        "rank(close)                       1.0000       1.0000"
    );
    assert_eq!(cm.to_string_table(), expected);
}

// ---------------------------------------------------------------------------
// corr / CorrMethod / CorrMatrix 访问器
// ---------------------------------------------------------------------------

#[test]
fn corr_pearson_and_spearman_and_kendall_basics() {
    let x = vec![1.0, 2.0, 3.0, 4.0, 5.0];
    let y = vec![2.0, 4.0, 6.0, 8.0, 10.0];
    assert!(approx(corr(&x, &y, CorrMethod::Pearson), 1.0));
    assert!(approx(corr(&x, &y, CorrMethod::Spearman), 1.0));
    assert!(approx(corr(&x, &y, CorrMethod::Kendall), 1.0));

    let yneg: Vec<f64> = x.iter().map(|v| -3.0 * v).collect();
    assert!(approx(corr(&x, &yneg, CorrMethod::Pearson), -1.0));
    assert!(approx(corr(&x, &yneg, CorrMethod::Kendall), -1.0));

    // 无并列的单调序 → τ 精确 ±1
    assert!(approx(
        corr(&[1.0, 2.0, 3.0], &[3.0, 2.0, 1.0], CorrMethod::Kendall),
        -1.0
    ));
    assert!(approx(
        corr(&[1.0, 2.0, 3.0], &[1.0, 2.0, 3.0], CorrMethod::Kendall),
        1.0
    ));
}

#[test]
fn corr_constant_or_short_inputs_yield_nan() {
    let x = vec![1.0, 2.0, 3.0];
    let c = vec![5.0, 5.0, 5.0];
    assert!(corr(&x, &c, CorrMethod::Pearson).is_nan());
    assert!(corr(&x, &c, CorrMethod::Kendall).is_nan()); // 全并列，分母为 0
    assert!(corr(&x, &c, CorrMethod::Spearman).is_nan());
    // 长度不足
    assert!(corr(&[1.0], &[1.0], CorrMethod::Pearson).is_nan());
    assert!(corr(&[1.0], &[1.0], CorrMethod::Kendall).is_nan());
    // 长度不等
    assert!(corr(&[1.0, 2.0], &[1.0], CorrMethod::Pearson).is_nan());
}

#[test]
fn corr_kendall_with_ties_matches_scipy() {
    // 与 scipy.kendalltau（pandas method='kendall' 的底层）同公式，并列只做分母修正，
    // tau-b = 0.8006407690254358（scipy 参考值）。
    let x = vec![1.0, 1.0, 2.0, 3.0, 3.0, 4.0];
    let y = vec![1.0, 2.0, 2.0, 2.0, 3.0, 4.0];
    let tau = corr(&x, &y, CorrMethod::Kendall);
    assert!(
        approx(tau, 0.800_640_769_025_435_8),
        "tau-b 与 scipy 一致，得 {tau}"
    );

    // 两侧完全反序 + 并列：scipy 给 ≈ -1
    let tau = corr(
        &[1.0, 1.0, 1.0, 2.0, 3.0],
        &[3.0, 3.0, 3.0, 2.0, 1.0],
        CorrMethod::Kendall,
    );
    assert!(approx(tau, -1.0), "反序并列 tau 应接近 -1，得 {tau}");
}

#[test]
fn corr_kendall_with_nan_yields_nan_not_panic() {
    // 三个口径遇 NaN 的行为要一致：Pearson / Spearman 由 NaN 自然传播，
    // Kendall 早先靠 sort 的 expect 兜底会 panic，现在同样返回 NaN
    let x = vec![1.0, 2.0, f64::NAN, 4.0];
    let y = vec![2.0, 1.0, 3.0, 4.0];
    for m in [
        CorrMethod::Pearson,
        CorrMethod::Spearman,
        CorrMethod::Kendall,
    ] {
        assert!(corr(&x, &y, m).is_nan(), "{m:?} 遇 NaN 应给 NaN");
        assert!(corr(&y, &x, m).is_nan(), "{m:?} 遇 NaN 应给 NaN（换边）");
    }
}

#[test]
fn corr_kendall_matches_brute_force_definition() {
    // 树状数组版（O(n log n)）与 tau-b 的定义式（O(n²) 枚举数对）逐值比对，
    // 刻意压低取值范围造出大量并列以覆盖 xtie / ytie / ntie 三种修正项
    fn brute(x: &[f64], y: &[f64]) -> f64 {
        let n = x.len();
        let (mut con, mut dis, mut xt, mut yt) = (0i64, 0i64, 0f64, 0f64);
        for i in 0..n {
            for j in (i + 1)..n {
                let (dx, dy) = (x[i] - x[j], y[i] - y[j]);
                if dx == 0.0 {
                    xt += 1.0;
                }
                if dy == 0.0 {
                    yt += 1.0;
                }
                let p = dx * dy;
                if p > 0.0 {
                    con += 1;
                } else if p < 0.0 {
                    dis += 1;
                }
            }
        }
        let n0 = (n * (n - 1) / 2) as f64;
        ((con - dis) as f64 / (n0 - xt).sqrt() / (n0 - yt).sqrt()).clamp(-1.0, 1.0)
    }

    let mut s = 0x2545_F491_4F6C_DD1Du64;
    let mut next = |m: u64| {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s % m
    };
    for (n, spread) in [(7usize, 3u64), (40, 5), (200, 7), (600, 400)] {
        let x: Vec<f64> = (0..n).map(|_| next(spread) as f64).collect();
        let y: Vec<f64> = (0..n).map(|_| next(spread) as f64).collect();
        let (fast, slow) = (corr(&x, &y, CorrMethod::Kendall), brute(&x, &y));
        assert!(
            approx(fast, slow) || (fast.is_nan() && slow.is_nan()),
            "n={n} spread={spread}：树状数组版 {fast} 与定义式 {slow} 不一致"
        );
    }
}

#[test]
fn correlation_kendall_diagonal_is_exactly_one() {
    // pandas 的 method='kendall' 走 nanops 逐对循环，把 i == j 硬编码成 1.0；
    // 本仓库在 correlation() 这一层照做，而 corr() 本身仍按 scipy 给 NaN
    let fa = panel_analyzer();
    let cm = fa.correlation(CorrMethod::Kendall);
    for i in 0..cm.len() {
        assert_eq!(cm.at(i, i), 1.0, "kendall 对角线应恰为 1.0");
    }
    let c = vec![5.0; 6];
    assert!(
        corr(&c, &c, CorrMethod::Kendall).is_nan(),
        "scipy 口径下常量列的 tau-b 是 NaN"
    );
}

#[test]
fn correlation_matrix_is_symmetric() {
    // 只算上三角再镜像：两侧入参互换在 IEEE 下逐位同值，故镜像后仍应逐位相等
    let fa = panel_analyzer();
    for m in [
        CorrMethod::Pearson,
        CorrMethod::Spearman,
        CorrMethod::Kendall,
    ] {
        let cm = fa.correlation(m);
        for i in 0..cm.len() {
            for j in 0..cm.len() {
                assert_eq!(cm.at(i, j).to_bits(), cm.at(j, i).to_bits(), "{m:?} 不对称");
            }
        }
    }
}

#[test]
fn method_parse_rejects_unknown() {
    assert_eq!(CorrMethod::parse("kendall").unwrap(), CorrMethod::Kendall);
    assert!(CorrMethod::parse("kendal").is_err());
    assert_eq!(IcMethod::parse("pearson").unwrap(), IcMethod::Pearson);
    // IC 不支持 kendall：上游会静默按 pearson，这里直接报错
    assert!(IcMethod::parse("kendall").is_err());
    // 上游若传 method 以外的值到 IC，会被静默当作 pearson；这里拒绝
    assert!(IcMethod::parse("none").is_err());
}

// ---------------------------------------------------------------------------
// summary
// ---------------------------------------------------------------------------

#[test]
fn summary_matches_upstream_text() {
    let fa = panel_analyzer();
    let expected = concat!(
        "FactorAnalyzer(factors=2, horizons=[1, 3])\n",
        "\n",
        "IC Analysis (Spearman):\n",
        "  Factor                      1D          3D\n",
        "  ------------------------------------------\n",
        "  signal(rank(close)      0.3423      0.2001\n",
        "  rank(close)             0.3423      0.2001\n",
        "\n",
        "IR (IC Mean / IC Std):\n",
        "  signal(rank(close)       0.531       0.366\n",
        "  rank(close)              0.531       0.366\n",
        "\n",
        "Factor Statistics:\n",
        "  Factor                Coverage    Turnover    Autocorr\n",
        "  ------------------------------------------------------\n",
        "  signal(rank(close)      83.33%      0.0536         N/A\n",
        "  rank(close)             83.33%      0.0536         N/A\n",
        "\n",
        "Correlation Matrix:\n",
        "                       signal(rank(close))  rank(close)\n",
        "  signal(rank(close))               1.0000       1.0000\n",
        "  rank(close)                       1.0000       1.0000"
    );
    assert_eq!(fa.summary(), expected);
}

#[test]
fn summary_single_factor_omits_correlation_section() {
    let panel = Panel::from_csv(PANEL).unwrap();
    let close = panel.factor("close").unwrap();
    let alpha = close.rank().signal();
    let fa = FactorAnalyzer::new(&[&alpha], &close, Some(&[1, 7, 30])).unwrap();
    let s = fa.summary();
    assert!(s.starts_with("FactorAnalyzer(factors=1, horizons=[1, 7, 30])"));
    assert!(!s.contains("Correlation Matrix"));
    // 默认持有期出现在 IC 表头
    assert!(s.contains("7D") && s.contains("30D"));
}

#[test]
fn summary_renders_nan_turnover_as_lowercase_nan() {
    // 上游 `f"{s['turnover']:.4f}".rjust(12)` 对 NaN 给的是 Python 的小写 nan，
    // 不是 Rust `{:.4}` 的 NaN；单期面板的 turnover 正是 NaN
    let one = factor_of(
        "one",
        &["2024-01-01"],
        &["A", "B", "C"],
        &[vec![1.0, 2.0, 3.0]],
    );
    let fa = FactorAnalyzer::new(&[&one], &one, Some(&[1])).unwrap();
    let s = fa.summary();
    // 与上游 `f"  {name}" + f"{cov:.2%}".rjust(12) + f"{to:.4f}".rjust(12) + ...` 同排版
    let row = format!("  {:<18}{:>12}{:>12}{:>12}", "one", "100.00%", "nan", "N/A");
    assert!(
        s.contains(&row),
        "turnover 应渲染成右对齐 12 字符的 nan，实际：\n{s}"
    );
}

#[test]
fn corr_matrix_accessors_out_of_range() {
    let fa = panel_analyzer();
    let cm = fa.correlation(CorrMethod::Kendall);
    assert_eq!(cm.method(), CorrMethod::Kendall);
    assert!(cm.at(5, 0).is_nan());
    assert_eq!(cm.get("nope", "rank(close)"), None);
    // Display for CorrMatrix 会输出 pandas 风格表格
    assert!(cm.to_string().contains("1.0000"));
}

// ---------------------------------------------------------------------------
// 校验：对 method 静默降级的上游差异作出说明性断言（防御上游不变）
// ---------------------------------------------------------------------------

#[test]
fn factor_ic_uses_original_values_for_pearson_vs_ranked_spearman() {
    // 用逐期第一条 ic 说明两者不同：spearman 0.5（对名次），pearson 0.8297（对原值）
    let fa = panel_analyzer();
    let sp = fa.ic(IcMethod::Spearman);
    let pe = fa.ic(IcMethod::Pearson);
    let s1 = &sp[0].by_horizon[0].ic_series;
    let p1 = &pe[0].by_horizon[0].ic_series;
    assert!(approx(s1[0].1, 0.5));
    assert!(approx(p1[0].1, 0.829_738_483_663_489_8));
}
