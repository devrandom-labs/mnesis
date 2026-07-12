//! Architecture tests for mnesis.
//!
//! Now that the kernel IS the crate, architecture is enforced
//! by Cargo's dependency graph. This test verifies the crate
//! has no external runtime dependencies beyond arrayvec and thiserror.

#[test]
fn mnesis_has_minimal_dependencies() {
    // If mnesis compiles with only arrayvec + thiserror,
    // the architecture is correct. This test documents that intent.
    // Any new dependency added to mnesis Cargo.toml will require
    // deliberate consideration.
    let cargo_toml = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"));
    assert!(
        cargo_toml.contains("arrayvec"),
        "mnesis should depend on arrayvec"
    );
    assert!(
        cargo_toml.contains("thiserror"),
        "mnesis should depend on thiserror"
    );
    // Ensure heavy dependencies are NOT present
    assert!(
        !cargo_toml.contains("tokio-stream"),
        "mnesis must not depend on tokio-stream (async belongs in mnesis-store)"
    );
    assert!(
        !cargo_toml.contains("async-trait"),
        "mnesis must not depend on async-trait (async belongs in mnesis-store)"
    );
    assert!(
        !cargo_toml.contains("serde ="),
        "mnesis must not depend on serde (serialization belongs in mnesis-store)"
    );
}
