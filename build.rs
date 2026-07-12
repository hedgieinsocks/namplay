use std::{env, path::PathBuf, process::Command};

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());

    let blp = manifest.join("data/window.blp");
    let ui = out.join("window.ui");

    let status = Command::new("blueprint-compiler")
        .args(["compile", blp.to_str().unwrap(), "--output", ui.to_str().unwrap()])
        .status()
        .expect("blueprint-compiler not found");
    assert!(status.success(), "blueprint-compiler failed");

    println!("cargo:rerun-if-changed=data/window.blp");
}
