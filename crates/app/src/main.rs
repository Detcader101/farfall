//! Native shell: the whole app lives in the library so the same code is the
//! web build's wasm module (`web/`). This is only the entry point.
fn main() {
    farfall_app::run();
}
