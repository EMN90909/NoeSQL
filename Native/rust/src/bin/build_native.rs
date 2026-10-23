use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

fn run(program: &str, args: &[&str], cwd: &Path) -> Result<ExitStatus, String> {
    let status = Command::new(program).args(args).current_dir(cwd).status()
        .map_err(|error| format!("failed to launch {program}: {error}"))?;
    if !status.success() { return Err(format!("{program} exited with {status}")); }
    Ok(status)
}

fn main() {
    if env::consts::OS != "linux" || env::consts::ARCH != "x86_64" {
        eprintln!("native Noqeri AOT integration is currently qualified only for Linux x86-64");
        std::process::exit(2);
    }
    let mut args = env::args().skip(1);
    let compiler = args.next().unwrap_or_else(|| {
        eprintln!("usage: build_native <noqeri-compiler> <bridge.noqeri> <output-dir>");
        std::process::exit(2);
    });
    let source = args.next().unwrap_or_else(|| {
        eprintln!("missing Noqeri source path");
        std::process::exit(2);
    });
    let output = PathBuf::from(args.next().unwrap_or_else(|| {
        eprintln!("missing output directory");
        std::process::exit(2);
    }));

    let source = fs::canonicalize(&source).unwrap_or_else(|error| {
        eprintln!("cannot resolve {source}: {error}");
        std::process::exit(1);
    });
    fs::create_dir_all(&output).unwrap_or_else(|error| {
        eprintln!("cannot create {}: {error}", output.display());
        std::process::exit(1);
    });
    let output = fs::canonicalize(&output).unwrap_or(output);
    let assembly = output.join("noesql_noqeri.s");
    let object = output.join("noesql_noqeri.o");
    let archive = output.join("libnoesql_noqeri.a");
    let cc = env::var("CC").unwrap_or_else(|_| "cc".into());
    let ar = env::var("AR").unwrap_or_else(|_| "ar".into());
    let cwd = env::current_dir().expect("current directory");

    let compiler_args = [
        "build",
        source.to_str().expect("UTF-8 source path"),
        assembly.to_str().expect("UTF-8 assembly path"),
        "-O2",
    ];
    if let Err(error) = run(&compiler, &compiler_args, &cwd) {
        eprintln!("{error}");
        std::process::exit(1);
    }

    let cc_args = [
        "-c",
        assembly.to_str().expect("UTF-8 assembly path"),
        "-o",
        object.to_str().expect("UTF-8 object path"),
    ];
    if let Err(error) = run(&cc, &cc_args, &cwd) {
        eprintln!("{error}");
        std::process::exit(1);
    }

    let ar_args = [
        "rcs",
        archive.to_str().expect("UTF-8 archive path"),
        object.to_str().expect("UTF-8 object path"),
    ];
    if let Err(error) = run(&ar, &ar_args, &cwd) {
        eprintln!("{error}");
        std::process::exit(1);
    }

    println!("{}", archive.display());
}
