//! The binary, driven against the in-process emulator.
//!
//! `-t sim` gives every invocation a fresh emulated switch, so these are single-shot: what the
//! tool prints, and what it exits with. Multi-step conversations are tested in the libraries,
//! where the clock can be paused.

use assert_cmd::Command;

/// `tesmart` with the emulator as its target and no colour, so output is comparable.
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

/// Standard output of a successful run.
fn stdout(args: &[&str]) -> String {
    let output = tesmart(args).assert().success();
    String::from_utf8(output.get_output().stdout.clone()).expect("UTF-8")
}

#[test]
fn switching_is_confirmed_on_a_switch_that_can_be_read() {
    let out = stdout(&["-t", "sim", "-m", "HKS1601-EB23", "switch", "4"]);
    insta::assert_snapshot!(out, @"switched to port 4
");
}

#[test]
fn switching_says_so_when_it_cannot_be_confirmed() {
    // The 4x1 DVI series documents four switch commands and nothing else.
    let out = stdout(&["-t", "sim://DKS0401A30", "-m", "DKS0401A30", "switch", "3"]);
    insta::assert_snapshot!(out, @"port 3 requested, not verified: reading the active port is unsupported on this model
");
}

#[test]
fn a_setting_reports_that_it_cannot_be_read_back() {
    let output = tesmart(&["-t", "sim", "-m", "HKS1601-EB23", "buzzer", "off"]).assert().success();

    let out = String::from_utf8(output.get_output().stdout.clone()).unwrap();
    insta::assert_snapshot!(out, @"buzzer sent, state cannot be read back
");
}

#[test]
fn an_unverified_capability_warns_on_standard_error() {
    let output = tesmart(&["-t", "sim://HKS801-M24", "-m", "HKS801-M24", "buzzer", "off"])
        .assert()
        .success();

    let stderr = String::from_utf8(output.get_output().stderr.clone()).unwrap();
    assert!(stderr.contains("warning: buzzer is unverified"), "{stderr}");
}

#[test]
fn reading_the_port_prints_just_the_port() {
    let out = stdout(&["-t", "sim", "-m", "HKS1601-EB23", "get"]);
    insta::assert_snapshot!(out, @"port 1
");
}

#[test]
fn a_matrix_reports_its_routing() {
    let out = stdout(&["-t", "sim://HMA0404B30", "-m", "HMA0404B30", "matrix", "show"]);
    insta::assert_snapshot!(out, @r"
    OUTPUT  INPUT
    1       1
    2       2
    3       3
    4       4
    (mirrored)
    ");
}

/// Routing is the one write in the protocol a switch will confirm.
#[test]
fn routing_a_matrix_is_confirmed() {
    let out = stdout(&["-t", "sim://HMA0404B30", "-m", "HMA0404B30", "matrix", "route", "3", "2"]);
    insta::assert_snapshot!(out, @r"
    routed, confirmed by the matrix
      output 1 <- input 1
      output 2 <- input 3
      output 3 <- input 3
      output 4 <- input 4
    ");
}

/// A 16x16 writes ports above 9 as A-G and answers in two segments; neither should show.
#[test]
fn a_16x16_matrix_reports_every_output() {
    let out = stdout(&[
        "-t",
        "sim://HMA1616A10",
        "-m",
        "HMA1616A10",
        "-o",
        "json",
        "matrix",
        "route",
        "16",
        "10",
    ]);
    assert!(out.contains(r#""output":10,"input":16"#), "{out}");
    assert!(out.contains(r#""output":16,"input":16"#), "{out}");
    assert!(out.contains(r#""verified":true"#), "{out}");
}

#[test]
fn a_matrix_preset_round_trips() {
    // Saving is write-only; loading reads the routing back.
    let out =
        stdout(&["-t", "sim://HMA0808A10", "-m", "HMA0808A10", "matrix", "preset", "load", "1"]);
    assert!(out.starts_with("preset 1 applied"), "{out}");
    assert!(out.contains("output 8 <- input 8"), "{out}");
}

#[test]
fn a_matrix_takes_its_network_settings_too() {
    let out = stdout(&["-t", "sim://HMA0808A10", "-m", "HMA0808A10", "net", "show"]);
    assert!(out.contains("192.168.1.10"), "{out}");
}

#[test]
fn json_is_one_object_with_a_stable_shape() {
    let out = stdout(&["-t", "sim", "-m", "HKS1601-EB23", "-o", "json", "switch", "4"]);
    insta::assert_snapshot!(out, @r#"{"ok":true,"command":"switch","target":"sim://HKS1601-EB23","model":"HKS1601-EB23","result":{"port":4,"verified":true,"already_active":false,"attempts":1}}
"#);
}

#[test]
fn json_carries_the_warnings_a_human_sees_on_stderr() {
    let out =
        stdout(&["-t", "sim://HKS801-M24", "-m", "HKS801-M24", "-o", "json", "buzzer", "off"]);
    assert!(out.contains(r#""warnings":["buzzer is unverified on this model"]"#), "{out}");
}

#[test]
fn json_failures_carry_a_class_a_script_can_switch_on() {
    let output =
        tesmart(&["-t", "sim", "-m", "DKS0401A30", "-o", "json", "switch", "8"]).assert().code(2);

    let out = String::from_utf8(output.get_output().stdout.clone()).unwrap();
    insta::assert_snapshot!(out, @r#"{"ok":false,"command":"switch","target":"sim://HKS1601-EB23","model":"DKS0401A30","error":{"class":"usage","message":"port 8 does not exist on a 4-port switch"}}
"#);
}

#[test]
fn quiet_prints_nothing_and_still_works() {
    let output =
        tesmart(&["-t", "sim", "-m", "HKS1601-EB23", "-q", "switch", "2"]).assert().success();

    assert_eq!(output.get_output().stdout, b"");
}

#[test]
fn the_network_settings_are_normalised_for_reading() {
    let out = stdout(&["-t", "sim", "-m", "HKS1601-EB23", "net", "show"]);
    insta::assert_snapshot!(out, @r"
    ip        192.168.1.10
    port      5000
    gateway   192.168.1.1
    mask      255.255.255.0
    ");
}

#[test]
fn probe_reports_what_the_registry_cannot() {
    let out = stdout(&["-t", "sim://HKS801-P23", "-m", "HKS801-P23", "probe"]);
    insta::assert_snapshot!(out, @r"
    CAPABILITY         REGISTRY     PROBE       DETAIL                       SUGGESTION
    read-active-port   unverified   works       port 1                       suggest: --assume read-port=yes
    net-query          unverified   works       192.168.1.10                 suggest: --assume net=yes
    buzzer             documented   not probed  write only, cannot be read back
    display-timeout    documented   not probed  write only, cannot be read back
    auto-input-detect  documented   not probed  write only, cannot be read back
    ");
}

#[test]
fn raw_bytes_go_out_and_the_answer_comes_back() {
    // The read-port command, by hand.
    let out = stdout(&["-t", "sim", "raw", "AA", "BB", "03", "10", "00", "EE", "--expect-reply"]);
    insta::assert_snapshot!(out, @r"
    sent  AA BB 03 10 00 EE
    reply port 1
    ");
}

#[test]
fn raw_needs_no_model() {
    // The one command that does not consult the opcode tables.
    tesmart(&["-t", "sim", "raw", "aabb03010 2ee"]).assert().success();
}

#[test]
fn the_registry_lists_what_it_knows_and_how_well() {
    let out = stdout(&["models", "list"]);
    assert!(out.starts_with("MODEL          PROTOCOL  PORTS  READ-PORT     NET\n"), "{out}");
    assert!(out.contains("DKS0401A30     kvm           4  unsupported   unverified"), "{out}");
    assert!(out.contains("HKS801-EB23    kvm           8  field-tested  field-tested"), "{out}");
    assert!(out.contains("dual-4port     kvm-dual      4  unsupported   unsupported"), "{out}");
}

#[test]
fn a_model_shows_its_opcodes_and_its_evidence() {
    let out = stdout(&["models", "show", "p23-8"]);
    assert!(out.starts_with("HKS801-P23 (kvm, 8 ports)\naliases: p23-8\n"), "{out}");
    assert!(
        out.contains("read-active-port       0x10    unverified    no evidence either way"),
        "{out}"
    );
    assert!(out.contains("no field report exists"), "the notes have to be printed: {out}");
}

#[test]
fn dual_monitor_commands_reach_a_dual_monitor_switch() {
    for args in [
        vec!["dual", "usb"],
        vec!["dual", "step", "a", "next"],
        vec!["dual", "passthrough", "off"],
        vec!["dual", "hotkey", "scroll-lock"],
        vec!["dual", "autoswitch", "on"],
        vec!["dual", "interval", "30s"],
    ] {
        let mut full = vec!["-t", "sim://dual-4port", "-m", "dual-4port"];
        full.extend(args.iter().copied());
        tesmart(&full).assert().success();
    }
}

#[test]
fn next_reads_and_switches_on_a_single_monitor_switch() {
    let out = stdout(&["-t", "sim", "-m", "HKS1601-EB23", "next"]);
    insta::assert_snapshot!(out, @"switched to port 2
");
}

#[test]
fn next_steps_on_a_dual_monitor_switch_and_says_it_cannot_confirm() {
    let out = stdout(&["-t", "sim://dual-4port", "-m", "dual-4port", "next"]);
    insta::assert_snapshot!(out, @"step-output sent, state cannot be read back
");
}

#[test]
fn an_assumption_makes_an_unverified_model_verifiable() {
    let out = stdout(&[
        "-t",
        "sim://HKS801-P23",
        "-m",
        "HKS801-P23",
        "--assume",
        "read-port=yes",
        "switch",
        "5",
    ]);
    insta::assert_snapshot!(out, @"switched to port 5
");
}

#[test]
fn the_wire_is_visible_at_two_vs() {
    let output = tesmart(&["-t", "sim", "-m", "HKS1601-EB23", "-vv", "get"]).assert().success();

    let stderr = String::from_utf8(output.get_output().stderr.clone()).unwrap();
    assert!(stderr.contains("AA BB 03 10 00 EE"), "the command should be dumped: {stderr}");
    assert!(stderr.contains("tesmart::wire"), "{stderr}");
}
