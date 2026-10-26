//! Reproducible, local Rust parser / read execution microbenchmark.
//! Not a production OLTP or Noqeri AOT benchmark. No fake speedup figures.
use noesql_runtime::reference::{parse, Database, Params};
use std::hint::black_box;
use std::time::Instant;

fn main() {
    const ROWS: usize = 500;
    const COMPILATIONS: usize = 20_000;
    const READS: usize = 2_000;
    let mut database = Database::default();
    let mut source = String::from("table bench {\n id:int key auto\n value:int required\n}\n");
    for value in 0..ROWS {
        source.push_str(&format!("bench add {{value:{value}}}\n"));
    }
    database.execute(&source, &Params::new()).expect("seed 500 rows");
    let query = "bench get id,value where value >= 490 sort id";
    let params = Params::new();

    let start = Instant::now();
    for _ in 0..COMPILATIONS {
        black_box(parse(black_box(query)).expect("parse query"));
    }
    let parse_ms = start.elapsed().as_secs_f64() * 1_000.0;

    let start = Instant::now();
    let mut produced = 0usize;
    for _ in 0..READS {
        produced += black_box(database.execute(black_box(query), &params).expect("read").rows.len());
    }
    let read_ms = start.elapsed().as_secs_f64() * 1_000.0;

    // Control is a deliberately *simulated* old-style full snapshot clone.
    // It is not a measured historic build. Run on the same machine only.
    let start = Instant::now();
    let mut cloned_produced = 0usize;
    for _ in 0..READS {
        let mut clone = database.clone();
        cloned_produced += black_box(clone.execute(query, &params).expect("cloned read").rows.len());
    }
    let clone_ms = start.elapsed().as_secs_f64() * 1_000.0;
    assert_eq!(produced, cloned_produced);

    println!("NoeSQL Rust-only parser + SELECT microbenchmark");
    println!("Rows: {ROWS}, parses: {COMPILATIONS}, reads: {READS}");
    println!("Parse: {parse_ms:.2} ms total ({:.0} parses/s)", COMPILATIONS as f64 * 1000.0 / parse_ms);
    println!("SELECT: {read_ms:.2} ms total ({:.0} reads/s)", READS as f64 * 1000.0 / read_ms);
    println!("SELECT + synthetic full clone: {clone_ms:.2} ms total ({:.0} reads/s)", READS as f64 * 1000.0 / clone_ms);
}
