fn main() {
    println!("cargo:rerun-if-changed=resources/windows.manifest");
    if std::env::var("TARGET").is_ok_and(|target| target.ends_with("windows-msvc")) {
        let root = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        println!("cargo:rustc-link-arg-bin=ccsw=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg-bin=ccsw=/MANIFESTINPUT:{root}/resources/windows.manifest");
    }
}
