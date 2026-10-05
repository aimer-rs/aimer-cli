use std::{env, process::Command};

fn main() {
    let rustc = env::var("RUSTC").expect("proc-macro-srv build expects RUSTC to be set");
    let output = Command::new(rustc)
        .arg("--version")
        .output()
        .expect("rustc --version must run");
    let version = std::str::from_utf8(&output.stdout)
        .expect("rustc --version output must be UTF-8")
        .trim();
    println!("cargo::rustc-env=RUSTC_VERSION={version}");
}
