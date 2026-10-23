use noesql_runtime::{read_source, validate_database_path};
use std::path::Path;

fn usage() -> ! {
    eprintln!("NoeSQL native runtime\n\nusage:\n  noesql version\n  noesql check <file.noe>\n  noesql check-db <file.noedb>");
    std::process::exit(2);
}

fn main() {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("version") => println!("NoeSQL native runtime 0.8.0"),
        Some("check") => {
            let path = args.next().unwrap_or_else(|| usage());
            match read_source(&path) {
                Ok(source) => println!("ok: {} bytes of UTF-8 NoeSQL source", source.len()),
                Err(error) => { eprintln!("error: {error}"); std::process::exit(1); }
            }
        }
        Some("check-db") => {
            let path = args.next().unwrap_or_else(|| usage());
            match validate_database_path(Path::new(&path)) {
                Ok(()) => println!("ok: canonical .noedb path"),
                Err(error) => { eprintln!("error: {error}"); std::process::exit(1); }
            }
        }
        _ => usage(),
    }
}
