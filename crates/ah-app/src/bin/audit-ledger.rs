use std::path::PathBuf;

fn main() {
    let mut args = std::env::args_os().skip(1);
    let input = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("audit/ledger.json"));
    let output = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("docs/generated/audit-summary.md"));
    if let Some(extra) = args.next() {
        eprintln!("unexpected argument: {}", extra.to_string_lossy());
        std::process::exit(2);
    }

    if let Err(error) = ah_app::audit::generate(&input, &output) {
        eprintln!("audit-ledger: {error}");
        std::process::exit(1);
    }
    println!("audit-ledger: generated {}", output.display());
}
