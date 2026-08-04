// Decode-only: parquet -> Arrow RecordBatches, no fp DataFrame conversion.
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
fn main() {
    let data = std::fs::read("/tmp/bench_num.parquet").unwrap();
    let mut best = u128::MAX;
    for _ in 0..6 {
        let b = bytes::Bytes::from(data.clone());
        let builder = ParquetRecordBatchReaderBuilder::try_new(b).unwrap();
        let nrows = builder.metadata().file_metadata().num_rows().max(0) as usize;
        let reader = builder.with_batch_size(nrows.max(1)).build().unwrap();
        let t = std::time::Instant::now();
        let mut rows = 0usize;
        for batch in reader {
            rows += batch.unwrap().num_rows();
        }
        std::hint::black_box(rows);
        best = best.min(t.elapsed().as_nanos());
    }
    println!("parquet DECODE-only 1Mx6: {:.2}ms", best as f64 / 1e6);
}
