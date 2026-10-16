//! Completions and the manual page.

use assert_cmd::Command;

fn tesmart(args: &[&str]) -> Command {
    let mut command = Command::cargo_bin("tesmart").expect("the binary is built");
    command.args(args);
    command
}

fn stdout(args: &[&str]) -> String {
    let output = tesmart(args).assert().success();
    String::from_utf8(output.get_output().stdout.clone()).expect("UTF-8")
}

#[test]
fn every_shell_gets_a_completion_script_naming_the_subcommands() {
    for shell in ["bash", "zsh", "fish", "powershell", "elvish"] {
        let script = stdout(&["completions", shell]);
        assert!(script.len() > 200, "{shell} script looks empty");
        assert!(script.contains("tesmart"), "{shell} script does not name the binary");
        assert!(script.contains("switch"), "{shell} script has no subcommands");
    }
}

#[test]
fn the_manual_page_documents_the_exit_codes() {
    let page = stdout(&["manpage"]);

    assert!(page.contains(".TH tesmart"), "not a roff page: {}", &page[..80.min(page.len())]);
    assert!(page.contains("Exit codes"), "the exit codes belong in the manual");
    assert!(page.contains("verification"), "{page}");
}

#[test]
fn the_manual_page_can_be_written_to_a_directory() {
    let directory = std::env::temp_dir().join("tesmart-manpage-test");
    std::fs::create_dir_all(&directory).expect("a temporary directory");

    let message = stdout(&["manpage", "--out", directory.to_str().unwrap()]);
    let written = directory.join("tesmart.1");

    assert!(message.contains("tesmart.1"), "{message}");
    assert!(written.is_file(), "{} was not written", written.display());
    assert!(std::fs::read_to_string(&written).unwrap().contains(".TH tesmart"));

    std::fs::remove_dir_all(&directory).ok();
}

#[test]
fn help_lists_the_exit_codes_so_they_can_be_relied_on() {
    let help = stdout(&["--help"]);

    for line in [
        "0  success",
        "2  usage",
        "3  transport",
        "4  timeout",
        "5  unsupported",
        "6  verification",
        "7  protocol",
    ] {
        assert!(help.contains(line), "`--help` is missing {line:?}");
    }
}

#[test]
fn help_shows_the_default_target_so_it_is_never_a_surprise() {
    let help = stdout(&["--help"]);
    assert!(help.contains("tcp://192.168.1.10:5000"), "{help}");
}
