fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Generate gRPC stubs from OpenTofu's tfplugin5.proto and tfplugin6.proto
    tonic_prost_build::configure().compile_protos(
        &[
            "vendor/proto/tfplugin5.proto",
            "vendor/proto/tfplugin6.proto",
        ],
        &["vendor/proto"],
    )?;

    println!("cargo:rerun-if-changed=vendor/proto/tfplugin5.proto");
    println!("cargo:rerun-if-changed=vendor/proto/tfplugin6.proto");

    Ok(())
}
