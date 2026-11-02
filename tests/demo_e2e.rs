//! Needs a window (macOS local). Run: cargo test --test demo_e2e -- --ignored
use std::path::Path;
use std::process::Command;

fn run_demo(script: &str, out: &Path) -> std::process::ExitStatus {
    let _ = std::fs::remove_dir_all(out);
    Command::new(env!("CARGO"))
        .args(["run", "--quiet", "--example", "demo"])
        .env("BCDT_SCRIPT", script)
        .env("BCDT_OUT", out)
        .env("BCDT_MAX_FRAMES", "2000")
        .status()
        .expect("cargo run")
}

#[test]
#[ignore]
fn drag_scenario_moves_square_and_records() {
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("e2e_drag");
    assert!(run_demo("scenarios/demo_drag.jsonl", &out).success());

    let pngs = std::fs::read_dir(out.join("frames")).unwrap().count();
    assert!(pngs >= 10, "only {pngs} frames");
    assert!(out.join("frames/final.png").exists(), "last screenshot lost on quit");

    let dump = std::fs::read_dir(out.join("dump")).unwrap().next().unwrap().unwrap().path();
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dump).unwrap()).unwrap();
    // window (600,150) on an 800x600 window → world (200, 150)
    let pos = &v["components"]["demo::Square"][0]["components"]["demo::Square"]["pos"];
    let (x, y) = (pos[0].as_f64().unwrap(), pos[1].as_f64().unwrap());
    assert!((x - 200.0).abs() < 1.0 && (y - 150.0).abs() < 1.0, "square at ({x},{y}): {v}");
}

#[test]
#[ignore]
fn bad_script_exits_2() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let script = dir.join("bad.jsonl");
    std::fs::write(&script, "{\"cmd\":\"wait\",\"frames\":1}\n{\"cmd\":\"bogus\"}\n").unwrap();
    let status = run_demo(script.to_str().unwrap(), &dir.join("e2e_bad"));
    assert_eq!(status.code(), Some(2));
}
