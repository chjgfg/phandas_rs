# phandas_rs

[![crates.io](https://img.shields.io/crates/v/phandas_rs.svg)](https://crates.io/crates/phandas_rs)
[![docs.rs](https://docs.rs/phandas_rs/badge.svg)](https://docs.rs/phandas_rs)
[![license](https://img.shields.io/crates/l/phandas_rs.svg)](./LICENSE)

Alpha factor construction, factor evaluation and event-driven backtesting on dense
panel data — 70+ cross-sectional and time-series operators, cross-sectional IC with
IR / t-stat, factor correlation and coverage / turnover / autocorr stats, portfolio
simulation with transaction costs, 11 performance metrics, zero dependencies. A Rust
port of the Python [phandas](https://github.com/quantbai/phandas) engine.

因子构造 + 因子评价 + 回测库：把 Python 版 phandas 的 `Factor` / `Panel` 与全部因子运算子、
因子评价器 `FactorAnalyzer`、以及事件驱动回测引擎移植到 Rust，**零外部依赖**（分位数反函数
与 CDF、最小二乘、条件数估计、CSV 解析、日期运算、相关系数全部自带实现）。

- [`factor`](https://docs.rs/phandas_rs/latest/phandas_rs/factor/) —— 因子构造，对应上游
  `core.py` / `operators.py` / `panel.py` / `constants.py`。
- [`backtest`](https://docs.rs/phandas_rs/latest/phandas_rs/backtest/) —— 事件驱动回测，
  对应上游 `backtest.py`（不含绘图）。
- [`analysis`](https://docs.rs/phandas_rs/latest/phandas_rs/analysis/) —— 因子评价，对应上游
  `analysis.py`（横截面 IC / IR / t 值、因子相关矩阵、覆盖率 / 换手率 / 自相关）。

上游的行情抓取、绘图与实盘下单未移植，它们各自绑定 ccxt / matplotlib / python-okx。

## 安装

```toml
[dependencies]
phandas_rs = "0.1"
```

## 快速开始

```rust
use phandas_rs::factor::{rank, vector_neut, Panel};

let csv = "\
timestamp,symbol,open,high,low,close,volume
2024-01-01,AAA,10,11,9,10,100
2024-01-01,BBB,20,21,19,20,200
2024-01-02,AAA,10,12,10,11,110
2024-01-02,BBB,20,22,18,19,190
";

let panel = Panel::from_csv_str(csv).unwrap();
let close = panel.factor("close").unwrap();
let volume = panel.factor("volume").unwrap();

// 1 期动量，再对成交量排名做向量投影中性化
let momentum = close.divide(&close.ts_delay(1)).subtract(1.0);
let alpha = vector_neut(&rank(&momentum), &rank(&volume.reverse()))
    .rename("momentum_1_neut");

println!("{}", alpha.show(10));
```

拿这个 alpha 跑回测，用次日开盘价成交：

```rust
use phandas_rs::backtest::Backtester;

let open = panel.factor("open").unwrap();
let signal = alpha.signal();                 // 归一成多空各 0.5 的美元中性权重
let bt = Backtester::new(&open, &signal)
    .transaction_cost(0.0003, 0.0003)
    .initial_capital(1000.0)
    .run()
    .unwrap()
    .calculate_metrics(0.03);

println!("{}", bt.summary());
println!("{}", bt.drawdown_report(5));
```

调仓是 T+1：第 `i` 期用第 `i-1` 期的因子值算目标市值，按第 `i` 期的成交价因子成交。
净值、收益率、回撤、换手率、等权基准都能单独取出（`bt.equity()` / `bt.returns()` /
`bt.drawdown()` / `bt.turnover()` / `bt.benchmark_equity()`），绘图交给调用方。
绩效指标需要至少两期收益率才算得出，上面那份 2 期的示例面板只够跑通流程。

拿候选因子与价格因子做因子评价——逐期横截面 IC、多持有期的 IR / t 值，因子相关矩阵
与覆盖率 / 换手率 / 自相关描述统计：

```rust
use phandas_rs::analysis::{analyze, CorrMethod, IcMethod};
use phandas_rs::factor::rank;

let factor_a = rank(&alpha); // alpha / close / volume 沿用上面的示例变量
let factor_b = rank(&volume);
let report = analyze(&[&factor_a, &factor_b], &close, None).unwrap(); // 默认持有期 [1, 7, 30]

println!("{}", report.summary());          // 与上游 summary() 逐字符对齐的排版
let ics = report.ic(IcMethod::Spearman);   // 每因子 × 每持有期：ic_mean / ic_std / ir / t_stat / 逐期序列
let cm = report.correlation(CorrMethod::Pearson);
println!(
    "alpha×volume 相关：{:.4}，共同样本 {} 格",
    cm.get("rank(momentum_1_neut)", "rank(volume)").unwrap(),
    cm.n_obs()
);
```

`ic_series` / `ic_stats`、相关系数 `corr` 与 `CorrMatrix` 都能脱离 `FactorAnalyzer` 单独使用，
数字口径逐格对齐上游 `analysis.py`（含 `ic_std` 用 numpy 的 `ddof = 0`、`turnover` 是
"先按标的取均值、再对标的取均值"的两级平均而非池化平均等细节）。Kendall 相关照 scipy 用
树状数组数反序对，`O(n log n)`——相关矩阵的样本是整块堆叠面板，朴素的逐对枚举撑不住。

跑内置演示与性能冒烟测试：

```bash
cargo run --bin phandas-demo
cargo run --release --example smoke -- path/to/crypto_1d.csv
```

## 数据模型

行 = `timestamp`（升序去重），列 = `symbol`（升序去重），值为 `f64`，缺失位置为 `NaN`。

- 横截面运算逐行计算，对应 Python 侧 `groupby('timestamp')`
- 时序运算逐列计算，对应 Python 侧 `groupby('symbol').rolling(...)`
- 二元运算按 `(timestamp, symbol)` 取交集，等价于 `pd.merge(how='inner')`

`Panel` 是共享同一索引的多列容器：`factor("close")` 取单列成 `Factor`、
`select(&["close", "volume"])` 取列子集、`slice_time` / `slice_symbols` 收窄索引、
`to_records()` / `to_csv_string()` / `to_csv(path)` 导出长表（NaN 写空字段）。

## 运算子

| 类别 | 举例 |
| --- | --- |
| 横截面 | `rank` `zscore` `normalize` `quantile` `scale` `spread` `signal` |
| 中性化 | `vector_neut` `regression_neut` `regression_neut_multi` `group_neutralize` |
| 分组 | `group_rank` `group_mean` `group_zscore` `group_scale` `group_normalize` |
| 时序 | `ts_delay` `ts_delta` `ts_rank` `ts_zscore` `ts_corr` `ts_regression` `ts_decay_linear` 等 65 个 |

完整清单见 [docs.rs](https://docs.rs/phandas_rs)。

## 回测

| 产出 | 方法 |
| --- | --- |
| 净值 / 收益率 / 回撤 | `equity()` `returns()` `drawdown()` |
| 换手率 / 成交流水 | `turnover()` `trades()` |
| 等权买入持有基准 | `benchmark_equity()` |
| 绩效指标（11 项） | `metrics()` —— 累计与年化收益、年化波动、夏普、索提诺、卡玛、最大回撤、净值线性度、VaR 95%、CVaR、PSR |
| 回撤区间明细 | `drawdown_periods()` |
| 文本报告 | `summary()` `drawdown_report(top_n)` |

口径与上游一致：年化按**日历天数** 365 折算（不是交易日），无风险利率默认 `0.03`，
双边手续费默认 `0.0003`。目标市值有三条路径——`Neutralization::None` 直接用因子值、
因子本身已是美元中性信号时直接用、否则去均值后按绝对值和归一。

## 与 Python 版的偏差

移植刻意保留了 Python 侧的若干反直觉行为（例如 `ts_decay_linear` 的权重方向、
`ts_skewness` 的复合定义、`spread` 中 NaN 占用多头名额），以保证因子值可比对。
另有少量已知偏差：`norm.ppf` 用 Acklam 有理逼近（相对误差约 `1e-9`）；时间戳按字符串
字典序排序，要求 ISO-8601 输入；`ts_corr` / `ts_covariance` / `ts_autocorr` 按定义正确
计算，未复刻上游的 groupby 错位；回测在 `full_rebalance` 模式下一天只记一条净值，
上游会记两条。逐条说明见
[`factor`](https://docs.rs/phandas_rs/latest/phandas_rs/factor/) 与
[`backtest`](https://docs.rs/phandas_rs/latest/phandas_rs/backtest/) 的模块文档。

## MSRV

Rust 1.82。

## License

MIT，见 [LICENSE](./LICENSE)。本 crate 是 [phandas](https://github.com/quantbai/phandas)
（MIT，Copyright (c) Phantom Management）的衍生作品，上游版权声明一并保留在 LICENSE 中。
