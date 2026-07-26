use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(
        env::var("CARGO_MANIFEST_DIR").expect("Cargo always supplies CARGO_MANIFEST_DIR"),
    );
    let schema_dir = manifest_dir.join("../../schemas");
    let schema = schema_dir.join("elevate.vexil");
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("Cargo always supplies OUT_DIR"));
    let vexilc = env::var("VEXILC_PATH").unwrap_or_else(|_| "vexilc".to_owned());

    let status = Command::new(vexilc)
        .arg("build")
        .arg(&schema)
        .arg("--include")
        .arg(&schema_dir)
        .arg("--output")
        .arg(&out_dir)
        .arg("--target")
        .arg("rust")
        .status()
        .unwrap_or_else(|error| panic!("failed to run vexilc: {error}"));

    if !status.success() {
        panic!("vexilc build failed for {}", schema.display());
    }

    println!("cargo::rerun-if-changed={}", schema.display());
    println!(
        "cargo::rerun-if-changed={}",
        schema_dir.join("common.vexil").display()
    );
}
