use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let local_lib = manifest_dir.join("lib");

    if local_lib.join("winfsp-x64.lib").exists() {
        println!("cargo:rustc-link-search={}", local_lib.display());
    }

    println!("cargo:rustc-link-search=C:\\Program Files (x86)\\WinFsp\\lib");

    // Ensure winfsp-x64.dll is present next to build output and test runners
    if let Ok(out_dir) = std::env::var("OUT_DIR") {
        let out_path = PathBuf::from(out_dir);
        if let Some(target_dir) = out_path.ancestors().nth(3) {
            let winfsp_dll = PathBuf::from("C:\\Program Files (x86)\\WinFsp\\bin\\winfsp-x64.dll");
            if winfsp_dll.exists() {
                let _ = std::fs::copy(&winfsp_dll, target_dir.join("winfsp-x64.dll"));
                let deps_dir = target_dir.join("deps");
                if deps_dir.exists() {
                    let _ = std::fs::copy(&winfsp_dll, deps_dir.join("winfsp-x64.dll"));
                }
            }
        }
    }
}
