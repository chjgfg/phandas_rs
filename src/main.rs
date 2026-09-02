use phandas_rs::factor::{rank, vector_neut, Panel};

fn main() {
    // 内置一个 3 期 × 3 标的的小面板，演示"数据 → 因子 → 展示"的完整链路
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
";

    let panel = match Panel::from_csv_str(csv) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("面板解析失败：{e}");
            return;
        }
    };

    let close = panel.factor("close").expect("close 列存在");
    let volume = panel.factor("volume").expect("volume 列存在");

    // 2 期动量，再对成交量排名做向量投影中性化
    let momentum = close.divide(&close.ts_delay(2)).subtract(1.0);
    let alpha = vector_neut(&rank(&momentum), &rank(&volume.reverse())).rename("momentum_2_neut");

    println!("{}", panel.info());
    println!();
    println!("{}", close.show(10));
    println!("{}", alpha.show(10));
}
