fn main() {
    tauri_build::build();

    #[cfg(windows)]
    {
        let resource = std::path::PathBuf::from(
            std::env::var_os("OUT_DIR").expect("Cargo must provide OUT_DIR"),
        )
        .join("resource.lib");
        println!("cargo:rustc-link-arg={}", resource.display());
    }
}
