# nix-c-raw

This crate contains generated bindings for the Nix C API.
**You should not have to use this crate directly,** and so you should probably not add it to your dependencies.
Instead, use the `nix-util`, `nix-store` and `nix-expr` crates, which _should_ be sufficient.

## Design

Rust bindgen currently does not allow "layered" libraries to be split into separate crates.
For example, the expr crate would have all-new types that are distinct and incompatible with the store crate.

Ideally bindgen will support reusing already generated modules, and we could move the code generation into the appropriate crates, so that the system dependencies of each crate become accurate.
