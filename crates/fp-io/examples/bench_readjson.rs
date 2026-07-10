use fp_io::JsonOrient;
fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^ (h >> 31)
}
fn gen_json(n: usize, salt: u64) -> String {
    let mut s = String::with_capacity(n * 60);
    s.push('[');
    for i in 0..n {
        if i > 0 {
            s.push(',');
        }
        let a = sm(i, 1 + salt) % 1_000_000;
        let b = (sm(i, 2 + salt) % 100000) as f64 / 100.0;
        let c = sm(i, 3 + salt) % 5000;
        s.push_str(&format!(
            "{{\"id\":{a},\"value\":{b},\"name\":\"item_{c}\"}}"
        ));
    }
    s.push(']');
    s
}
fn main() {
    let n = 300_000usize;
    let inputs: Vec<String> = (0..6).map(|k| gen_json(n, k as u64 * 7919)).collect();
    let bytes = inputs[0].len();
    let mut best = u128::MAX;
    for inp in &inputs {
        let t = std::time::Instant::now();
        let df = fp_io::read_json_str(inp, JsonOrient::Records).unwrap();
        let mut acc = 0usize;
        for name in df.column_names() {
            acc += df.column(name.as_str()).unwrap().values().len();
        }
        std::hint::black_box(acc);
        best = best.min(t.elapsed().as_nanos());
    }
    println!(
        "read_json 300kx3 records COLD: {:.2}ms ({:.1}MB)",
        best as f64 / 1e6,
        bytes as f64 / 1e6
    );
}
