use chrono::Local;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let build_date = Local::now().format("%d-%m-%Y (%H:%M:%S)").to_string();
    println!("cargo:rustc-env=AIMER_BUILD_TIME={build_date}");

    if let Err(error) = bundle_proc_macro_server() {
        panic!("could not bundle the Rust Analyzer proc-macro server: {error}");
    }
}

fn bundle_proc_macro_server() -> Result<(), String> {
    const HELPER_PACKAGE: &str = "tools/rust-analyzer-proc-macro-srv";
    const HELPER_MANIFEST: &str = "tools/rust-analyzer-proc-macro-srv/Cargo.toml";

    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_BUNDLED_PROC_MACRO");
    println!("cargo:rerun-if-env-changed=RUSTC");
    println!("cargo:rerun-if-changed={HELPER_PACKAGE}");
    println!("cargo:rerun-if-changed=vendor/aimer-rust-analyzer-proc-macro-srv");

    let out_dir = PathBuf::from(
        env::var_os("OUT_DIR").ok_or_else(|| "Cargo did not set OUT_DIR".to_owned())?,
    );
    let generated = out_dir.join("aimer_proc_macro_server.rs");

    if env::var_os("CARGO_FEATURE_BUNDLED_PROC_MACRO").is_none() {
        fs::write(
            generated,
            "const BUNDLED_PROC_MACRO_SERVER: Option<&'static [u8]> = None;\n",
        )
        .map_err(|error| format!("writing generated feature configuration: {error}"))?;
        return Ok(());
    }

    let manifest_dir = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR")
            .ok_or_else(|| "Cargo did not set CARGO_MANIFEST_DIR".to_owned())?,
    );
    let target = env::var("TARGET").map_err(|error| format!("reading TARGET: {error}"))?;
    let host = env::var("HOST").map_err(|error| format!("reading HOST: {error}"))?;
    let helper_target_dir = out_dir.join("proc_macro_server_target");
    let cargo = env::var_os("CARGO").ok_or_else(|| "Cargo did not set CARGO".to_owned())?;

    println!("cargo:warning=Building the bundled proc-macro helper for {target}.");
    let mut command = Command::new(cargo);
    command
        .current_dir(&manifest_dir)
        .arg("build")
        .arg("--release")
        .arg("--locked")
        .arg("--manifest-path")
        .arg(manifest_dir.join(HELPER_MANIFEST))
        .arg("--target-dir")
        .arg(&helper_target_dir)
        .env("RUSTC_BOOTSTRAP", "1");
    if target != host {
        command.arg("--target").arg(&target);
    }

    let output = command
        .output()
        .map_err(|error| format!("starting the helper build: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "helper build failed ({}):\n{}{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let executable_suffix = match env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("windows") => ".exe",
        _ => "",
    };
    let executable_name = format!("rust-analyzer-proc-macro-srv{executable_suffix}");
    let mut helper_dir = helper_target_dir;
    if target != host {
        helper_dir.push(&target);
    }
    helper_dir.push("release");
    let built_helper = helper_dir.join(&executable_name);
    if !built_helper.is_file() {
        return Err(format!(
            "Cargo succeeded but did not produce {}",
            built_helper.display()
        ));
    }

    let bundled_helper = out_dir.join(&executable_name);
    fs::copy(&built_helper, &bundled_helper)
        .map_err(|error| format!("copying helper from {}: {error}", built_helper.display()))?;
    fs::write(
        generated,
        format!(
            "const BUNDLED_PROC_MACRO_SERVER: Option<&'static [u8]> = Some(include_bytes!(concat!(env!(\"OUT_DIR\"), \"/{executable_name}\")));\n"
        ),
    )
    .map_err(|error| format!("writing generated helper asset declaration: {error}"))?;

    println!("cargo:warning=Bundled the matching Rust Analyzer proc-macro helper.");
    Ok(())
}
