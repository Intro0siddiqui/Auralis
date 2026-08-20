fn main() {
    if let Ok(target) = std::env::var("TARGET") {
        if target.contains("android") {
            println!("cargo:rustc-link-arg=-Wl,-z,max-page-size=16384");
            println!("cargo:rustc-link-arg=-Wl,-z,common-page-size=16384");

            if let Ok(ndk) =
                std::env::var("NDK_HOME").or_else(|_| std::env::var("ANDROID_NDK_HOME"))
            {
                let arch = if target.starts_with("aarch64") {
                    "aarch64-linux-android"
                } else if target.starts_with("armv7") || target.starts_with("arm") {
                    "arm-linux-androideabi"
                } else if target.starts_with("x86_64") {
                    "x86_64-linux-android"
                } else {
                    "i686-linux-android"
                };

                let host_dirs = [
                    "linux-x86_64",
                    "darwin-x86_64",
                    "darwin-arm64",
                    "windows-x86_64",
                ];
                for host in &host_dirs {
                    for api in (26..=36).rev() {
                        let p = std::path::PathBuf::from(&ndk)
                            .join("toolchains/llvm/prebuilt")
                            .join(host)
                            .join("sysroot/usr/lib")
                            .join(arch)
                            .join(api.to_string());
                        if p.exists() {
                            println!("cargo:rustc-link-search={}", p.display());
                        }
                    }
                }
            }
        }
    }
    tauri_build::build()
}
