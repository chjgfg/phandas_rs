//! 端到端流水线示范：**抓数 → 造因子 → 评价 → 回测 → 实盘**。
//!
//! 五步全在本仓库内，不需要 MCP、不需要任何外部脚本。跑法：
//!
//! ```text
//! # 只跑前四步（抓数 / 造因子 / 评价 / 回测），不碰交易所账户
//! cargo run --release --features data --example pipeline
//!
//! # 加上第五步：把最后一期的因子权重变成 OKX 模拟盘的目标仓位
//! #   先在仓库根的 .env 里填好三行 OKX_* 凭证（照 .env.example 抄）
//! cargo run --release --features data,trader --example pipeline -- --trade
//! ```
//!
//! **第五步默认只出计划不发单。** 真要发单再加 `--execute`，且这个示范写死打**模拟盘**
//! （`Environment::Demo`）——要打实盘得自己改代码，改的时候你会看见那一行。

use phandas_rs::analysis::{analyze, CorrMethod, IcMethod};
use phandas_rs::backtest::Backtester;
use phandas_rs::data::{fetch_data, Source, Timeframe};
use phandas_rs::factor::{rank, vector_neut, Factor, Panel};

/// 研究用的标的池。
const SYMBOLS: [&str; 6] = ["ETH", "SOL", "ARB", "OP", "POL", "SUI"];

#[tokio::main]
async fn main() {
    // 本地要走代理就把 HTTPS_PROXY 写进 .env；服务器上不放这个文件即直连
    let n = phandas_rs::net::load_default_dotenv();
    println!("[env] .env 写入 {n} 条\n");

    if let Err(e) = run().await {
        eprintln!("\n流水线中断：{e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let panel = step1_fetch().await?;
    let (alpha, momentum) = step2_factors(&panel)?;
    step3_analyze(&panel, &alpha, &momentum)?;
    step4_backtest(&panel, &alpha)?;
    step5_trade(&alpha).await?;
    Ok(())
}

/// ① 抓数：Binance 日线 OHLCV，拼成一张 `Panel`。
///
/// 只用 `Source::Binance` 是为了让示范跑得快：每个标的一年日线就一批请求。
/// 加 `Source::Vwap` 会贵很多——日线的 VWAP 是内部抓 1h 再按自然日聚合的，
/// 一年就是 8760 根、每标的九批；`Source::Benchmark` 另加 BTC / ETH 两趟。
async fn step1_fetch() -> Result<Panel, String> {
    println!("① 抓数 ——————————————————————————————————————————");
    let panel = fetch_data(
        &SYMBOLS,
        Timeframe::D1,
        Some("2024-01-01"),
        Some("2024-12-31"),
        &[Source::Binance],
    )
    .await?;
    println!("{}\n", panel.info());
    Ok(panel)
}

/// ② 造因子：20 期动量，再对成交量排名做向量投影中性化。
///
/// 返回 `(中性化后的 alpha, 中性化前的动量)`——两个都留着，第③步要比它们的相关性。
fn step2_factors(panel: &Panel) -> Result<(Factor, Factor), String> {
    println!("② 造因子 ——————————————————————————————————————");
    let close = panel.factor("close")?;
    let volume = panel.factor("volume")?;

    // 运算子可以任意链式复合，因子名会跟着自动积累
    let momentum = close.divide(&close.ts_delay(20)).subtract(1.0);
    let alpha = vector_neut(&rank(&momentum), &rank(&volume.reverse())).rename("mom20_neut_vol");

    println!("动量因子：{}", momentum.name);
    println!("中性化后：{}", alpha.name);
    println!("{}\n", alpha.show(6));
    Ok((alpha, rank(&momentum)))
}

/// ③ 评价：逐期横截面 IC、多持有期的 IR / t 值，再看两个因子重不重。
fn step3_analyze(panel: &Panel, alpha: &Factor, momentum: &Factor) -> Result<(), String> {
    println!("③ 评价 ——————————————————————————————————————————");
    let close = panel.factor("close")?;
    let report = analyze(&[alpha, momentum], &close, Some(&[1, 7, 30]))?;
    println!("{}\n", report.summary());

    // 相关矩阵能看出中性化到底改了多少
    let cm = report.correlation(CorrMethod::Pearson);
    if let Some(r) = cm.get(&alpha.name, &momentum.name) {
        println!("中性化前后相关：{r:.4}（共同样本 {} 格）", cm.n_obs());
    }

    // 单独取 IC 序列自己做进一步分析
    let ics = report.ic(IcMethod::Spearman);
    if let Some(s) = ics[0].for_horizon(7) {
        println!(
            "alpha 7 日 IC：均值 {:.4}、IR {:.3}、t 值 {:.2}、有效期数 {}\n",
            s.ic_mean,
            s.ir,
            s.t_stat,
            s.ic_series.len()
        );
    }
    Ok(())
}

/// ④ 回测：把因子归一成美元中性权重，按次日开盘价成交。
fn step4_backtest(panel: &Panel, alpha: &Factor) -> Result<(), String> {
    println!("④ 回测 ——————————————————————————————————————————");
    let open = panel.factor("open")?;
    let signal = alpha.signal(); // 多空各 0.5、总和 0

    let bt = Backtester::new(&open, &signal)
        .transaction_cost(0.0003, 0.0003)
        .initial_capital(10_000.0)
        .run()?
        .calculate_metrics(0.03);

    println!("{}", bt.summary());
    println!("{}", bt.drawdown_report(3));
    // 净值 / 回撤 / 换手率都能单独取出来自己画图
    println!(
        "净值 {} 点，平均换手率 {:.4}\n",
        bt.equity().len(),
        {
            let t = bt.turnover();
            if t.is_empty() {
                0.0
            } else {
                t.iter().map(|(_, v)| v).sum::<f64>() / t.len() as f64
            }
        }
    );
    Ok(())
}

/// ⑤ 实盘：把最后一期的因子权重变成 OKX 的目标仓位。
///
/// `Factor::to_weights` 是「研究世界」与「实盘世界」唯一的接口——它把因子矩阵某一期的
/// 横截面抽成 `{标的: 权重}`，再乘预算就是目标名义额。
///
/// 默认**只出计划不发单**；加 `--execute` 才真发，且写死打模拟盘。
#[cfg(feature = "trader")]
async fn step5_trade(alpha: &Factor) -> Result<(), String> {
    use phandas_rs::trader::{Confirm, Environment, OkxTrader, Rebalancer};

    let args: Vec<String> = std::env::args().collect();
    if !args.iter().any(|a| a == "--trade") {
        println!("⑤ 实盘 —— 跳过（加 --trade 才走这一步）");
        // 即便不连交易所，也能先看看权重长什么样
        let w = alpha.signal().to_weights(None);
        println!("   最后一期的目标权重：{w:?}\n");
        return Ok(());
    }
    println!("⑤ 实盘 ——————————————————————————————————————————");

    // 写死模拟盘。要打实盘得把这里改成 Environment::Live——改的时候你会看见这一行
    let trader = OkxTrader::from_env(Environment::Demo)?;
    println!("环境：{:?}（is_live = {}）", trader.environment(), trader.environment().is_live());

    // 预算取账户总权益
    let budget = trader.balance().await?.total_equity;
    println!("账户总权益：${budget:.2}");

    // signal() 归一后多空各 0.5，正好当目标权重用
    let weights = alpha.signal().to_weights(None);
    let plan = Rebalancer::new(weights, budget)?
        .leverage(3)
        .plan(&trader)
        .await?;

    // 建计划只读，绝不发单
    println!("{}\n", plan.preview());

    if args.iter().any(|a| a == "--execute") {
        println!("发单中……（模拟盘）");
        let report = plan.execute(&trader, Confirm::Yes).await?;
        println!("{}\n", report.summary());
    } else {
        println!("以上只是计划。真要发单加 --execute。\n");
    }
    Ok(())
}

/// 没开 `trader` feature 时的占位，让示范在 `--features data` 下也能跑通前四步。
#[cfg(not(feature = "trader"))]
async fn step5_trade(alpha: &Factor) -> Result<(), String> {
    println!("⑤ 实盘 —— 未开启 trader feature，跳过");
    let w = alpha.signal().to_weights(None);
    println!("   最后一期的目标权重：{w:?}");
    println!("   要下单请用：cargo run --release --features data,trader --example pipeline -- --trade\n");
    Ok(())
}
