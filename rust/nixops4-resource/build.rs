use std::{env, fs, path::Path};

use typify::{TypeSpace, TypeSpaceSettings};

fn main() {
    let content = std::fs::read_to_string("resource-schema-v0.json").unwrap();
    let schema = serde_json::from_str::<schemars::schema::RootSchema>(&content).unwrap();

    let mut type_space = TypeSpace::new(
        TypeSpaceSettings::default()
            .with_derive("Debug".to_string())
            .with_derive("PartialEq".to_string())
            .with_derive("Eq".to_string())
            .with_derive("Clone".to_string())
            .with_map_type("std::collections::BTreeMap".to_string()),
    );
    type_space.add_ref_types(schema.definitions).unwrap();

    let contents =
        prettyplease::unparse(&syn::parse2::<syn::File>(type_space.to_stream()).unwrap());

    let mut out_path = Path::new(&env::var("OUT_DIR").unwrap()).to_path_buf();
    out_path.push("generated");
    fs::create_dir_all(out_path.clone()).unwrap();
    out_path.push("v0.rs");
    fs::write(out_path, contents).unwrap();
}
