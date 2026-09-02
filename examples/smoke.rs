use phandas_rs::factor::{rank, vector_neut, Panel};

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("用法: cargo run --example smoke -- <crypto_1d.csv>");
    let t0 = std::time::Instant::now();
    let panel = Panel::from_csv(&path).expect("CSV 可解析");
    println!("加载耗时 {:?}", t0.elapsed());
    println!("{}", panel.info());

    let close = panel.factor("close").expect("close");
    let open = panel.factor("open").expect("open");
    let volume = panel.factor("volume").expect("volume");

    let t1 = std::time::Instant::now();
    let momentum = close.divide(&close.ts_delay(20)).subtract(1.0);
    let alpha = vector_neut(&rank(&momentum), &rank(&volume.reverse()));
    println!("因子计算耗时 {:?}", t1.elapsed());
    println!("{}", alpha.info());

    let t2 = std::time::Instant::now();
    let heavy = close
        .ts_zscore(30)
        .add(&close.ts_corr(&volume, 30))
        .add(&close.ts_regression(&[&open], 30, 0, 6))
        .add(&close.ts_kurtosis(30))
        .add(&close.ts_decay_linear(10, false));
    println!("重算子组合耗时 {:?}", t2.elapsed());
    println!("{}", heavy.info());
    println!("{}", alpha.show(6));
}
