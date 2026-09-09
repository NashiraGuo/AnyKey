use anykey_engine::config::Config;
fn main() {
    let json = std::fs::read_to_string("../anykey_config.json").unwrap();
    let c: Config = serde_json::from_str(&json).unwrap();
    let td = &c.layers;
    // config 文件: holdTerm=200, doubleTapTerm=150, doubleHoldTerm=150
    // 若读到 250/200 → 是默认值 fallback（未匹配上 JSON 字段）
    println!("hold_term={} dtt={} dht={}", td.hold_term, td.double_tap_term, td.double_hold_term);
    // 直接 dump JSON 里的值做对比
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    println!("JSON tapDance timing: {:?}", &v["tapDance"]["holdTerm"]);
    println!("JSON tapDance timing: {:?}", &v["tapDance"]["doubleTapTerm"]);
    println!("JSON tapDance timing: {:?}", &v["tapDance"]["doubleHoldTerm"]);
}
