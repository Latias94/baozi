#![allow(
    clippy::result_large_err,
    reason = "facade examples preserve complete bounded terminal evidence by value"
)]

use baozi::{Error, ImportCause, Importer, Result};

fn main() -> Result<()> {
    let bytes = b"o warned
v 0 0 0
v 1 0 0
v 0 1 0
curv 0 1 1 2
f 1 2 3
";
    let importer = Importer::new()?;
    let report = importer.read_bytes("warned.obj", bytes)?;
    for diagnostic in report.diagnostics() {
        println!(
            "{:?} {} {}",
            diagnostic.severity(),
            diagnostic.code().as_str(),
            diagnostic.message()
        );
    }

    let malformed = b"v 0 0 0
f 1 2
";
    match importer.read_bytes("malformed.obj", malformed) {
        Ok(_) => println!("malformed import unexpectedly succeeded"),
        Err(Error::Import(failure)) if matches!(failure.cause(), ImportCause::Format(_)) => {
            println!("fatal format error: {}", failure.cause())
        }
        Err(error) => println!("fatal import error: {error}"),
    }

    Ok(())
}
