#![cfg(target_os = "linux")]

use std::process::Command;

#[test]
fn help_lists_agent_commands_without_audio_access() {
    let output = Command::new(env!("CARGO_BIN_EXE_clearwave"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    for command in ["doctor", "devices", "run", "volume", "default", "--json"] {
        assert!(text.contains(command), "missing {command}");
    }
}

#[test]
fn invalid_volume_is_json_and_nonzero_before_accessing_audio() {
    let output = Command::new(env!("CARGO_BIN_EXE_clearwave"))
        .args(["volume", "output", "nonexistent-device", "151", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["ok"], false);
    assert!(error["error"].as_str().unwrap().contains("151"));
    assert!(output.stdout.is_empty());
}

#[test]
fn unknown_command_is_json_and_nonzero() {
    let output = Command::new(env!("CARGO_BIN_EXE_clearwave"))
        .args(["--json", "unknown-command"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["ok"], false);
}
