//! Generate compiled plugin constructors from Cargo metadata, not runtime directories.
fn main() {
    println!("cargo:rerun-if-changed=Cargo.toml");
    let manifest: toml::Value = std::fs::read_to_string("Cargo.toml")
        .unwrap()
        .parse()
        .unwrap();
    let mut output = String::from("fn compiled_plugins() -> Vec<std::sync::Arc<dyn yuantuan_plugin_api::Plugin>> {\n #[allow(unused_mut)] let mut plugins = Vec::new();\n");
    if let Some(entries) = manifest["package"]["metadata"]["yuantuan"]["plugins"].as_array() {
        for entry in entries {
            let dependency = entry["dependency"].as_str().expect("plugin dependency");
            let feature = entry["feature"].as_str().expect("plugin feature");
            for name in [dependency, feature] {
                assert!(
                    !name.is_empty()
                        && name
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
                    "invalid plugin name"
                );
            }
            assert!(
                manifest["dependencies"].get(dependency).is_some(),
                "plugin dependency missing"
            );
            assert!(
                manifest["features"].get(feature).is_some(),
                "plugin feature missing"
            );
            output.push_str(&format!(
                "#[cfg(feature = {feature:?})] plugins.push({}::plugin());\n",
                dependency.replace('-', "_")
            ));
        }
    }
    output.push_str("plugins\n}\n");
    std::fs::write(
        std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("plugins.rs"),
        output,
    )
    .unwrap();
}
