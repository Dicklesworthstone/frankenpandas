//! Read benchmarks. bench_read <csv|json> <path>
use fp_io::JsonOrient;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let kind = a.get(1).map(String::as_str).unwrap_or("csv");
    let path = a.get(2).map(String::as_str).unwrap_or("/tmp/bench.csv");
    let input = std::fs::read_to_string(path).unwrap();
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        let rows = match kind {
            "jsonl" => {
                let df = fp_io::read_jsonl_str(&input).unwrap();
                df.index().len()
            }
            "json" => {
                let df = fp_io::read_json_str(&input, JsonOrient::Records).unwrap();
                df.index().len()
            }
            _ => {
                let df = fp_io::read_csv_str(&input).unwrap();
                // Force full column materialization, not just lazy index len.
                let mut acc = 0usize;
                for name in df.column_names() {
                    acc += df.column(name.as_str()).unwrap().values().len();
                }
                acc
            }
        };
        std::hint::black_box(rows);
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("read_{kind}: best={best}ns ({:.2}ms)", best as f64 / 1e6);
}
