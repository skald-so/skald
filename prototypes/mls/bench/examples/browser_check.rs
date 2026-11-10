//! A WASM module that runs the browser's GroupInfo check on both prototypes at every size.

use mls_bench::{Budget, Operation, SIZES, run, times_table};
use mls_proto_mls_rs::MlsRsDevice;
use mls_proto_openmls::OpenMlsDevice;
use wasm_bindgen::prelude::wasm_bindgen;

/// The times of the browser's GroupInfo check as a Markdown table.
#[wasm_bindgen]
pub fn browser_check() -> Result<String, String> {
    let operations = [Operation::BrowserCheck];
    let results = [
        run::<OpenMlsDevice>(&SIZES, &operations, Budget::FULL, |_| {}),
        run::<MlsRsDevice>(&SIZES, &operations, Budget::FULL, |_| {}),
    ]
    .into_iter()
    .collect::<Result<Vec<_>, _>>()
    .map_err(|error| error.to_string())?;
    Ok(times_table(&results))
}
