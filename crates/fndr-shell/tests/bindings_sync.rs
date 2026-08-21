//! The committed bindings must match what the current Rust types generate.
//! This is the "at build time" half of T-105: CI fails when a type or command
//! changes without regenerating.

use std::fs;
use std::path::Path;

#[test]
fn bindings_in_sync() {
    let tmp = std::env::temp_dir().join(format!("fndr-bindings-{}.ts", std::process::id()));
    fndr_shell::export_bindings(&tmp).expect("export failed");
    let generated = fs::read_to_string(&tmp).expect("read generated");
    let _ = fs::remove_file(&tmp);

    let committed_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ui/src/bindings/bindings.ts");
    let committed = fs::read_to_string(&committed_path)
        .expect("ui/src/bindings/bindings.ts missing; run scripts/gen-bindings.sh");

    assert_eq!(
        generated, committed,
        "ui/src/bindings/bindings.ts is stale; run scripts/gen-bindings.sh and commit"
    );
}
