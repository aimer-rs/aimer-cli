use std::path::Path;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=RUSTC");
    let rustc = std::env::var_os("RUSTC").expect("Cargo sets RUSTC for build scripts");
    let output = Command::new(rustc)
        .args(["--print", "sysroot"])
        .output()
        .expect("rustc --print sysroot must run");
    assert!(output.status.success(), "rustc --print sysroot failed");
    let sysroot = String::from_utf8(output.stdout).expect("sysroot path must be UTF-8");
    let library_dir = Path::new(sysroot.trim()).join("lib");
    println!("cargo:rustc-link-search=native={}", library_dir.display());

    #[cfg(unix)]
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", library_dir.display());
}
