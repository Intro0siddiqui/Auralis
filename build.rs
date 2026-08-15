fn main() {
    if let Ok(target) = std::env::var("TARGET") {
        if target.contains("android") {
            println!("cargo:rustc-link-arg=-Wl,-z,max-page-size=16384");
            println!("cargo:rustc-link-arg=-Wl,-z,common-page-size=16384");
        }
    }
    tauri_build::build()
}
