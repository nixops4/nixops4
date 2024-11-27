use std::env;
use std::path::PathBuf;

#[derive(Debug)]
struct StripNixPrefix {}
impl bindgen::callbacks::ParseCallbacks for StripNixPrefix {
    fn item_name(&self, name: &str) -> Option<String> {
        name.strip_prefix("nix_").map(String::from)
    }
}

fn main() {
    // Tell cargo to invalidate the built crate whenever the wrapper changes
    println!("cargo:rerun-if-changed=include/nix-c-raw.h");
    println!("cargo:rustc-link-lib=nixflake");

    // https://rust-lang.github.io/rust-bindgen/library-usage.html
    let bindings = bindgen::Builder::default()
        .header("include/nix-c-raw.h")
        // Find the includes
        .clang_args(c_headers())
        // Tell cargo to invalidate the built crate whenever any of the
        // included header files changed.
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .parse_callbacks(Box::new(StripNixPrefix {}))
        // Finish the builder and generate the bindings.
        .generate()
        // Unwrap the Result and panic on failure.
        .expect("Unable to generate bindings");

    // Write the bindings to the $OUT_DIR/bindings.rs file.
    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");
}

fn c_headers() -> Vec<String> {
    let mut args = Vec::new();
    // args.push("-isystem".to_string());
    for path in pkg_config::probe_library("nix-flake-c")
        .unwrap()
        .include_paths
        .iter()
    {
        args.push(format!("-I{}", path.to_str().unwrap()));
    }

    for path in pkg_config::probe_library("bdw-gc")
        .unwrap()
        .include_paths
        .iter()
    {
        args.push(format!("-I{}", path.to_str().unwrap()));
    }

    // write to stderr for debugging
    eprintln!("c_headers: {:?}", args);
    args
}
