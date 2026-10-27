// Build script: compiles the .slint UI into Rust code at build time
// (no UI files are parsed at runtime) and links the Windows resources
// (app icon + manifest) that were pre-compiled into res/.

fn main() {
    // Fluent = the Windows 11 look for the settings window widgets.
    let config = slint_build::CompilerConfiguration::new().with_style("fluent".into());
    slint_build::compile_with_config("ui/app.slint", config).expect("Slint UI failed to compile");

    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_os == "windows" {
        let dir = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("res");
        // Pre-built resource files, so building needs no resource compiler:
        //   islo.res   -> MSVC linker
        //   islo_res.o -> GNU linker (MinGW)
        let file = if target_env == "msvc" { "islo.res" } else { "islo_res.o" };
        let path = dir.join(file);
        if path.exists() {
            println!("cargo:rustc-link-arg-bins={}", path.display());
        }
        println!("cargo:rerun-if-changed=res");
    }
    embed_package();
}

/// Bake package\ (the notifications manifest and its logos) into Islo.exe, so
/// Settings can write it out and register it: Islo.exe is the only file users need.
fn embed_package() {
    let root = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("package");
    let mut files = Vec::new();
    let mut dirs = vec![root.clone()];
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let path = e.path();
            if path.is_dir() {
                dirs.push(path);
            } else {
                let rel = path.strip_prefix(&root).unwrap().to_string_lossy().replace('/', "\\");
                files.push((rel, path));
            }
        }
    }
    files.sort();
    let mut code = String::from("pub const FILES: &[(&str, &[u8])] = &[\n");
    for (rel, path) in &files {
        code += &format!("    ({rel:?}, include_bytes!({:?})),\n", path.to_string_lossy());
    }
    code += "];\n";
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("package_files.rs");
    std::fs::write(out, code).unwrap();
    println!("cargo:rerun-if-changed=package");
}
