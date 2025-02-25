fn main() {
    // Should be more incremental
    // blocked on https://github.com/rust-lang/rust/issues/99515
    // and its implementation in schemafy
    println!("cargo:rerun-if-changed=resource-schema-v0.json");
}
