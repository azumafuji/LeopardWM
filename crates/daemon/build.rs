use std::{env, fs, path::PathBuf};

fn embed_locales() {
    println!("cargo:rerun-if-changed=locales");
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let mut catalogs = fs::read_dir(root.join("locales"))
        .expect("Failed to read locale directory")
        .map(|entry| entry.expect("Failed to read locale entry").path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension == "toml")
        })
        .collect::<Vec<_>>();
    catalogs.sort();
    let mut table = String::from("const BUNDLED_CATALOGS: &[(&str, &str)] = &[\n");
    for path in catalogs {
        let identifier = path
            .file_stem()
            .unwrap()
            .to_str()
            .expect("Locale filename must be UTF-8");
        let path = path.to_str().expect("Locale path must be UTF-8");
        table.push_str(&format!("({identifier:?}, include_str!({path:?})),\n"));
    }
    table.push_str("];\n");
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("locales.rs");
    fs::write(output, table).expect("Failed to generate embedded locales");
}

fn main() {
    embed_locales();
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../assets/leopardwm.ico");
    res.set_manifest(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
</assembly>"#,
    );
    res.compile().expect("Failed to compile Windows resources");
}
