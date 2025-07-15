use std::{env, fs, path::Path};

use typify::{TypeSpace, TypeSpaceSettings};

fn main() {
    schema_to_rust(
        "resource-schema-v0.json",
        "generated/schema/resource",
        "v0.rs",
    );
    schema_to_rust("state-schema-v0.json", "generated/schema/state", "v0.rs");
}

fn schema_to_rust(schema_file: &str, out_dir: &str, out_file: &str) {
    let content = std::fs::read_to_string(schema_file).unwrap();
    let schema = serde_json::from_str::<schemars::schema::RootSchema>(&content).unwrap();

    let mut type_space = TypeSpace::new(
        TypeSpaceSettings::default()
            .with_derive("Debug".to_string())
            .with_derive("PartialEq".to_string())
            .with_derive("Eq".to_string())
            .with_derive("Clone".to_string())
            .with_crate(
                "json_patch",
                typify::CrateVers::parse("4.0.0").unwrap(),
                None,
            )
            .with_map_type("std::collections::BTreeMap".to_string()),
    );
    type_space.add_ref_types(schema.definitions).unwrap();

    let contents =
        prettyplease::unparse(&syn::parse2::<syn::File>(type_space.to_stream()).unwrap());

    let mut out_path = Path::new(&env::var("OUT_DIR").unwrap()).to_path_buf();
    out_path.push(out_dir);
    fs::create_dir_all(out_path.clone()).unwrap();
    out_path.push(out_file);
    fs::write(out_path, contents).unwrap();
}
