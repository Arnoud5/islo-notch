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
}
