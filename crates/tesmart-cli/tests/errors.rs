//! The exit-code contract.
//!
//! These codes are documented in `--help` and in the manual page, so they are part of the
//! tool's interface: a script switching on them must keep working.

use assert_cmd::Command;

fn tesmart(args: &[&str]) -> Command {
    let mut command = Command::cargo_bin("tesmart").expect("the binary is built");
    command
        .env_remove("TESMART_TARGET")
        .env_remove("TESMART_MODEL")
        .env_remove("TESMART_LOG")
        .arg("--color")
        .arg("never")
        .args(args);
    command
}

/// Runs and returns the exit code and standard error.
fn fails(args: &[&str], code: i32) -> String {
    let output = tesmart(args).assert().code(code);
    String::from_utf8(output.get_output().stderr.clone()).expect("UTF-8")
}

#[test]
fn bad_arguments_are_clap_s_exit_code_two() {
    tesmart(&["switch"]).assert().code(2);
    tesmart(&["timeout", "20s"]).assert().code(2);
    tesmart(&["--nonsense"]).assert().code(2);
}

#[test]
fn a_missing_model_explains_both_ways_out() {
    let stderr = fails(&["-t", "sim", "get"], 2);
    assert!(stderr.contains("pass -m <MODEL>"), "{stderr}");
    assert!(stderr.contains("-p kvm --ports 8"), "{stderr}");
    // And why, because the reason is the whole point of refusing.
    assert!(stderr.contains("0x03"), "{stderr}");
}

#[test]
fn an_unknown_model_points_at_the_registry() {
    let stderr = fails(&["-t", "sim", "-m", "HKS9999", "get"], 2);
    assert!(stderr.contains("tesmart models list"), "{stderr}");
}

#[test]
fn a_port_the_switch_does_not_have_is_a_usage_error() {
    let stderr = fails(&["-t", "sim://DKS0401A30", "-m", "DKS0401A30", "switch", "8"], 2);
    assert!(stderr.contains("port 8 does not exist on a 4-port switch"), "{stderr}");
}

#[test]
fn a_switch_that_cannot_be_reached_is_three() {
    let stderr = fails(&["-t", "127.0.0.1:1", "-m", "HKS801-EB23", "get"], 3);
    assert!(stderr.contains("tcp://127.0.0.1:1"), "{stderr}");
}

#[test]
fn a_command_this_model_lacks_is_five() {
    // The first generation 8x1 documents switching and reading, nothing else.
    let stderr = fails(&["-t", "sim://HKS0801A20", "-m", "HKS0801A20", "buzzer", "off"], 5);
    assert!(stderr.contains("buzzer is unsupported on HKS0801A20"), "{stderr}");
}

#[test]
fn a_dual_monitor_command_on_a_single_monitor_switch_is_five() {
    fails(&["-t", "sim", "-m", "HKS1601-EB23", "dual", "usb"], 5);
}

#[test]
fn require_verified_turns_an_unconfirmable_result_into_six() {
    // Without the flag this is a success: the command was sent and nothing more can be known.
    tesmart(&["-t", "sim://DKS0401A30", "-m", "DKS0401A30", "switch", "3"]).assert().success();

    let stderr = fails(
        &["-t", "sim://DKS0401A30", "-m", "DKS0401A30", "--require-verified", "switch", "3"],
        6,
    );
    assert!(stderr.contains("could not be confirmed"), "{stderr}");
}

#[test]
fn a_switch_that_ignores_commands_fails_verification_with_six() {
    // Nothing in the emulator ignores commands over `-t sim`, so this is the next best thing:
    // a model whose read-back disagrees is tested in the client. Here, check the class is
    // wired to the right code by way of a mismatched network write.
    let stderr = fails(
        &["-t", "sim", "-m", "HKS1601-EB23", "--require-verified", "switch", "4", "--no-verify"],
        6,
    );
    assert!(stderr.contains("verification was disabled"), "{stderr}");
}

#[test]
fn writing_network_settings_needs_yes_and_something_to_write() {
    let stderr = fails(&["-t", "sim", "-m", "HKS1601-EB23", "net", "set", "--ip", "10.0.2.12"], 2);
    assert!(stderr.contains("--yes"), "{stderr}");
    assert!(stderr.contains("no factory reset"), "{stderr}");

    let stderr = fails(&["-t", "sim", "-m", "HKS1601-EB23", "net", "set", "--yes"], 2);
    assert!(stderr.contains("nothing to set"), "{stderr}");
}

#[test]
fn an_unknown_target_scheme_says_what_is_expected() {
    let stderr = fails(&["-t", "udp://192.168.1.10", "-m", "HKS801-EB23", "get"], 2);
    assert!(stderr.contains("expected tcp://, serial:// or sim"), "{stderr}");
}

#[test]
fn a_bad_assumption_says_what_is_accepted() {
    let stderr = fails(&["-t", "sim", "-m", "HKS801-P23", "--assume", "read-port", "get"], 2);
    assert!(stderr.contains("KIND=SUPPORT"), "{stderr}");

    let stderr =
        fails(&["-t", "sim", "-m", "HKS801-P23", "--assume", "read-port=probably", "get"], 2);
    assert!(stderr.contains("support level"), "{stderr}");
}

#[test]
fn bad_hex_is_rejected_before_anything_is_sent() {
    let stderr = fails(&["-t", "sim", "raw", "AABBC"], 2);
    assert!(stderr.contains("pairs"), "{stderr}");
}

#[test]
fn a_matrix_is_not_a_kvm() {
    // A KVM command against a matrix model: refused, and the message says which to use.
    let stderr = fails(&["-t", "sim", "-m", "HMA0404B30", "get"], 2);
    assert!(stderr.contains("expected a KVM switch"), "{stderr}");
    assert!(stderr.contains("matrix protocol"), "{stderr}");
}

#[test]
fn a_kvm_is_not_a_matrix() {
    // And the other way round.
    let stderr = fails(&["-t", "sim", "-m", "HKS801-EB23", "matrix", "show"], 2);
    assert!(stderr.contains("expected an HDMI matrix"), "{stderr}");
    assert!(stderr.contains("kvm protocol"), "{stderr}");
}

#[test]
fn a_matrix_command_this_series_lacks_is_refused() {
    // The 8x8 document has no MT00CT command, so this never reaches the wire.
    let stderr =
        fails(&["-t", "sim://HMA0808A10", "-m", "HMA0808A10", "matrix", "timeout", "30"], 5);
    assert!(stderr.contains("matrix-display-timeout"), "{stderr}");

    // And a 4x4 has no presets.
    let stderr =
        fails(&["-t", "sim://HMA0404B30", "-m", "HMA0404B30", "matrix", "preset", "save", "1"], 5);
    assert!(stderr.contains("matrix-save-preset"), "{stderr}");
}

#[test]
fn a_preset_slot_a_matrix_does_not_have_is_refused() {
    let stderr =
        fails(&["-t", "sim://HMA0808A10", "-m", "HMA0808A10", "matrix", "preset", "load", "9"], 2);
    assert!(stderr.contains("preset 9"), "{stderr}");
}
