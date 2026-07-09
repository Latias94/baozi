use baozi::{BaoziErrorKind, ImportOptions, Importer, Result};

fn main() -> Result<()> {
    let bytes = b"o warned
v 0 0 0
v 1 0 0
v 0 1 0
curv 0 1 1 2
f 1 2 3
";
    let report = Importer::new().read_bytes("warned.obj", bytes)?;
    for diagnostic in report.diagnostics() {
        println!(
            "{:?} {} {}",
            diagnostic.severity, diagnostic.code.0, diagnostic.message
        );
    }

    let mut strict = ImportOptions::memory();
    strict.diagnostics.strict = true;
    match Importer::new().read_bytes_with_options("strict.obj", bytes, strict) {
        Ok(_) => println!("strict import unexpectedly succeeded"),
        Err(error) => println!("strict import failed as {:?}: {}", error.kind(), error),
    }

    let malformed = b"v 0 0 0
f 1 2
";
    match Importer::new().read_bytes("malformed.obj", malformed) {
        Ok(_) => println!("malformed import unexpectedly succeeded"),
        Err(error) if error.kind() == BaoziErrorKind::Parse => {
            println!("fatal parse error: {error}")
        }
        Err(error) => println!("fatal import error: {error}"),
    }

    Ok(())
}
