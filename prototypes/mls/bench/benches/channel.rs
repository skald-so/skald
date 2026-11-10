//! The benchmarks natively on both prototypes at every size: prints the times and, without the
//! `--times` argument, the object sizes as Markdown tables, and what runs next to stderr.

use mls_bench::{Budget, Operation, SIZES, run, sizes_table, times_table};
use mls_proto_mls_rs::MlsRsDevice;
use mls_proto_openmls::OpenMlsDevice;

fn main() -> mls_scenario::Result<()> {
    let progress = |next: &str| eprintln!("bench: {next}");
    let results = [
        run::<OpenMlsDevice>(&SIZES, &Operation::ALL, Budget::FULL, progress)?,
        run::<MlsRsDevice>(&SIZES, &Operation::ALL, Budget::FULL, progress)?,
    ];
    print!("{}", times_table(&results));
    if !std::env::args().any(|arg| arg == "--times") {
        print!("\n{}", sizes_table(&results));
    }
    Ok(())
}
