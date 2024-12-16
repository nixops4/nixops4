fn main() {
    // Get nix version
    let nix_version = pkg_config::probe_library("nix-store-c").unwrap().version;

    // Generate version flags
    // Unfortunately, Rust doesn't give us a "greater than" operator in conditional
    // compilation, so we pre-evaluate the version comparisons here, making use
    // of the multi-valued nature of Rust cfgs.
    let relevant_versions = vec!["2.26"];
    let versions = relevant_versions
        .iter()
        .map(|v| format!("\"{}\"", v))
        .collect::<Vec<_>>()
        .join(",");

    // Declare the known versions, so that Rust can warn about unknown versions
    // that aren't part of `relevant_versions` yet - feel free to add entries.
    println!(
        "cargo:rustc-check-cfg=cfg(nix_at_least,values({}))",
        versions
    );

    let nix_version = nix_version.split('.').collect::<Vec<&str>>();
    let nix_version = (
        nix_version[0].parse::<u32>().unwrap(),
        nix_version[1].parse::<u32>().unwrap(),
    );

    for version_str in relevant_versions {
        let version = version_str.split('.').collect::<Vec<&str>>();
        let version = (
            version[0].parse::<u32>().unwrap(),
            version[1].parse::<u32>().unwrap(),
        );
        if nix_version >= version {
            println!("cargo:rustc-cfg=nix_at_least=\"{}\"", version_str);
        }
    }
}
