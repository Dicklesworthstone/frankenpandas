fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^ (h >> 31)
}
fn gen_csv(n: usize, salt: u64) -> String {
    let mut s = String::with_capacity(n * 40);
    s.push_str("i0,f0,i1,f1,name\n");
    for i in 0..n {
        let i0 = if sm(i, 9 + salt).is_multiple_of(10) {
            String::new()
        } else {
            (sm(i, 1 + salt) % 1_000_000).to_string()
        };
        let f0 = if sm(i, 8 + salt).is_multiple_of(10) {
            String::new()
        } else {
            format!("{:.2}", (sm(i, 2 + salt) % 100000) as f64 / 100.0)
        };
        let i1 = if sm(i, 7 + salt).is_multiple_of(10) {
            String::new()
        } else {
            (sm(i, 3 + salt) % 900000).to_string()
        };
        let f1 = if sm(i, 6 + salt).is_multiple_of(10) {
            String::new()
        } else {
            format!("{:.1}", (sm(i, 4 + salt) % 50000) as f64 / 10.0)
        };
        let c = sm(i, 5 + salt) % 5000;
        s.push_str(&format!("{i0},{f0},{i1},{f1},item_{c}\n"));
    }
    s
}
fn main() {
    let n = 500_000usize;
    let inputs: Vec<String> = (0..6).map(|k| gen_csv(n, k as u64 * 7919)).collect();
    let bytes = inputs[0].len();
    let mut best = u128::MAX;
    for inp in &inputs {
        let t = std::time::Instant::now();
        let df = fp_io::read_csv_str(inp).unwrap();
        let mut acc = 0usize;
        for name in df.column_names() {
            acc += df.column(name.as_str()).unwrap().values().len();
        }
        std::hint::black_box(acc);
        best = best.min(t.elapsed().as_nanos());
    }
    println!(
        "read_csv 500kx5 (4 nullable-num + 1 str) COLD: {:.2}ms ({:.1}MB)",
        best as f64 / 1e6,
        bytes as f64 / 1e6
    );
}
