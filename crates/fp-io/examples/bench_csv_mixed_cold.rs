// Cold mixed-CSV parse bench (500k x 3: i64, f64, utf8). Distinct input per iter
// so the parse cache never hits (memory: measure COLD).
fn sm(i: usize, s: u64) -> u64 { let mut h=(i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15); h=(h^(h>>30)).wrapping_mul(0xBF58476D1CE4E5B9); h^(h>>31) }
fn gen_csv(n: usize, salt: u64) -> String {
    let mut s = String::with_capacity(n*36);
    s.push_str("id,value,name\n");
    for i in 0..n {
        let a = sm(i, 1+salt) % 1_000_000;
        let b = (sm(i, 2+salt) % 100000) as f64 / 100.0;
        let c = sm(i, 3+salt) % 5000;
        s.push_str(&format!("{a},{b:.2},item_{c}\n"));
    }
    s
}
fn main(){
    let n = 500_000usize;
    // Pre-generate distinct inputs so generation isn't in the timed section.
    let inputs: Vec<String> = (0..6).map(|k| gen_csv(n, k as u64 * 7919)).collect();
    let bytes = inputs[0].len();
    let mut best = u128::MAX;
    for inp in &inputs {
        let t = std::time::Instant::now();
        let df = fp_io::read_csv_str(inp).unwrap();
        std::hint::black_box(df.index().len());
        best = best.min(t.elapsed().as_nanos());
    }
    println!("read_csv mixed 500kx3 COLD: {:.2}ms ({:.1}MB)", best as f64/1e6, bytes as f64/1e6);
}
