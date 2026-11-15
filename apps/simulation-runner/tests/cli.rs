use std::path::PathBuf;
use std::process::Command;

fn runner_cmd() -> Command {
    let default_path = PathBuf::from(env!("CARGO_BIN_EXE_simulation-runner"));
    if default_path.exists() {
        return Command::new(default_path);
    }

    // Fallback: If mounted into different container directories (e.g. /app vs /src),
    // resolve relative to the test binary's output location (target/debug/deps -> target/debug).
    if let Some(target_dir) = std::env::current_exe().ok().and_then(|exe| {
        exe.parent()
            .and_then(|p| p.parent())
            .map(|p| p.to_path_buf())
    }) {
        let candidate = target_dir.join("simulation-runner");
        if candidate.exists() {
            return Command::new(candidate);
        }
    }

    Command::new(default_path)
}

#[test]
fn check_mode_does_not_claim_simulation() {
    let out = runner_cmd()
        .args(["--seed", "42", "--ticks", "10000", "--check"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["seed"], 42);
    assert_eq!(report["executed_ticks"], 0);
    assert_eq!(report["simulation_available"], false);
    assert_eq!(report["status"], "configuration_valid");
}

#[test]
fn invalid_ticks_fail() {
    let out = runner_cmd()
        .args(["--ticks", "0", "--check"])
        .output()
        .unwrap();
    assert!(!out.status.success());
}

#[test]
fn live_simulation_executes_and_produces_deterministic_report() {
    let out1 = runner_cmd()
        .args(["--seed", "9999", "--ticks", "50"])
        .output()
        .unwrap();
    assert!(out1.status.success());
    let report1: serde_json::Value = serde_json::from_slice(&out1.stdout).unwrap();
    assert_eq!(report1["seed"], 9999);
    assert_eq!(report1["executed_ticks"], 50);
    assert_eq!(report1["simulation_available"], true);
    assert_eq!(report1["status"], "completed");

    let out2 = runner_cmd()
        .args(["--seed", "9999", "--ticks", "50"])
        .output()
        .unwrap();
    assert!(out2.status.success());
    let report2: serde_json::Value = serde_json::from_slice(&out2.stdout).unwrap();

    assert_eq!(
        report1["final_state_hash"], report2["final_state_hash"],
        "CLI runs with identical seed must output identical final_state_hash"
    );
}
