//! Standalone repeat-query microbenchmark with correctness checks; does
//! not certify production performance. Run in release mode on one machine.
use noesql_runtime::reference::{Database, Params, Row};
use noesql_runtime::value::Value;
use std::time::Instant;
fn main() {
    const ROWS: usize = 30_000;
    const QUERIES: usize = 500;
    let mut db = Database::default();
    db.execute("table events { id:int key auto\n owner:int required\n payload:int required }\nindex owner_idx on events(owner)",
        &Params::new()).unwrap();
    let table = db.tables.get_mut("events").unwrap();
    for index in 0..ROWS {
        let mut row = Row::new();
        row.insert("id".into(), Value::Int(index as i64 + 1));
        row.insert("owner".into(), Value::Int((index % 10_000) as i64));
        row.insert("payload".into(), Value::Int(index as i64));
        table.rows.push(row);
    }
    // This benchmark populates rows directly before any query cache is built.
    let mut p = Params::new();
    p.insert("owner".to_string(), Value::Int(999));
    let indexed = "events get id,payload where owner == $owner";
    // Verify indexed path against an equivalent scan with a non-indexable
    // condition, then exclude the lazy-build step from warm-query timing.
    let warm = db.execute(indexed, &p).unwrap().rows;
    let scan = db.execute("events get id,payload where owner >= 999 && owner <= 999", &p)
        .unwrap().rows;
    assert_eq!(warm, scan, "index changed result semantics");
    let start = Instant::now();
    for _ in 0..QUERIES { assert_eq!(db.execute(indexed, &p).unwrap().rows.len(), 3); }
    let indexed_elapsed = start.elapsed();
    let start = Instant::now();
    for _ in 0..QUERIES {
        assert_eq!(db.execute("events get id,payload where owner >= 999 && owner <= 999", &p)
            .unwrap().rows.len(), 3);
    }
    let scan_elapsed = start.elapsed();
    println!("rows={ROWS} queries={QUERIES} bplus_warm_ms={:.3} full_scan_ms={:.3}",
        indexed_elapsed.as_secs_f64()*1000., scan_elapsed.as_secs_f64()*1000.);
    println!("comparison is illustrative, not an IOPS or durable v3 page-index benchmark");
}
