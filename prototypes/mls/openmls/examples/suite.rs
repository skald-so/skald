//! A WASM module with the whole scenario suite on OpenMLS, for its size.

use wasm_bindgen::prelude::wasm_bindgen;

/// Runs every scenario; panics on the first that fails.
#[wasm_bindgen]
pub fn run_suite() {
    mls_scenario::run_scenarios!(mls_proto_openmls::OpenMlsDevice);
}
