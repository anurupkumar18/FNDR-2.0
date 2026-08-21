//! Writes the generated TypeScript bindings into ui/src/bindings/.
//! Run via scripts/gen-bindings.sh; the bindings_in_sync test keeps the
//! committed file honest.

use std::path::Path;

fn main() {
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ui/src/bindings/bindings.ts");
    fndr_shell::export_bindings(&out).expect("bindings export failed");
    println!("wrote {}", out.display());
}
