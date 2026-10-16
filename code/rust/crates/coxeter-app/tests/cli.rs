//! The command line: `--version`, and `--screenshot`, which draws one frame
//! offscreen (no window) with the files given opened.

mod common;

use std::process::Command;

fn coxeter() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_coxeter"));
    c.env("COXETER_ROOT", common::repo_root());
    c
}

#[test]
fn version() {
    let out = coxeter().arg("--version").output().unwrap();
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), format!("Coxeter-MFV {}", env!("CARGO_PKG_VERSION")));
}

#[test]
fn a_screenshot_with_a_model_opened() {
    let dir = common::temp_dir("cli");
    let png = dir.join("shot.png");
    let model = common::repo_root().join("models").join("ecoli_core_mc.json");
    let out = coxeter().args(["--screenshot"]).arg(&png).args(["--size", "1280x800", "--dark"]).arg(&model).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let image = image::open(&png).expect("a PNG");
    assert_eq!((image.width(), image.height()), (1280, 800));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("No installed font draws"), "{stderr}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_bad_option_says_how_to_call_it() {
    let out = coxeter().arg("--nonsense").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).starts_with("usage: coxeter"));
}
