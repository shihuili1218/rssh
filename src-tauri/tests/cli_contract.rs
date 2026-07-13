#![cfg(feature = "cli")]

use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn rssh(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rssh-cli"))
        .args(args)
        .output()
        .expect("run rssh CLI")
}

fn rssh_in_home(home: &Path, args: &[&str], input: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rssh-cli"))
        .args(args)
        .env("HOME", home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rssh CLI");
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(input.as_bytes())
        .expect("write CLI input");
    child.wait_with_output().expect("wait for rssh CLI")
}

fn assert_typed_completion_tree(shell: &str, legacy_tree: &str) {
    let home = tempfile::tempdir().expect("temporary HOME");
    let output = rssh_in_home(home.path(), &["completions", shell], "");
    assert!(
        output.status.success(),
        "{shell} stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("completion script is UTF-8");
    for command in [
        "profile",
        "credential",
        "forward",
        "group",
        "github",
        "webdav",
    ] {
        assert!(
            stdout.contains(command),
            "{shell} completion is missing {command}:\n{stdout}"
        );
    }
    assert!(
        !stdout.contains(legacy_tree),
        "{shell} completion still contains the legacy top-level tree:\n{stdout}"
    );
}

#[test]
fn root_help_exposes_only_typed_command_families() {
    let output = rssh(&["--help"]);
    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("help is UTF-8");
    for family in ["profile", "credential", "forward", "group"] {
        assert!(
            stdout
                .lines()
                .any(|line| line.starts_with(&format!("  {family}"))),
            "missing {family} command in:\n{stdout}"
        );
    }
    for legacy in ["ls", "open", "add", "edit", "rm", "_names"] {
        assert!(
            !stdout
                .lines()
                .any(|line| line.starts_with(&format!("  {legacy}"))),
            "legacy {legacy} command still present in:\n{stdout}"
        );
    }
}

#[test]
fn legacy_top_level_commands_are_rejected_by_clap() {
    for legacy in ["ls", "open", "add", "edit", "rm", "_names"] {
        let output = rssh(&[legacy]);
        assert_eq!(
            output.status.code(),
            Some(2),
            "{legacy} stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("unrecognized subcommand"),
            "{legacy} stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn bash_completions_follow_the_typed_command_tree() {
    assert_typed_completion_tree(
        "bash",
        "compgen -W \"ls open add edit rm config completions\"",
    );
}

#[test]
fn zsh_completions_follow_the_typed_command_tree() {
    assert_typed_completion_tree("zsh", "'ls:List profiles, credentials, or forwards'");
}

#[test]
fn fish_completions_follow_the_typed_command_tree() {
    assert_typed_completion_tree(
        "fish",
        "__fish_use_subcommand' -a 'ls' -d 'List profiles/credentials/forwards'",
    );
}

#[test]
fn powershell_completions_follow_the_typed_command_tree() {
    assert_typed_completion_tree(
        "powershell",
        "@('ls','open','add','edit','rm','config','completions')",
    );
}

#[test]
fn completions_do_not_require_a_database_home() {
    let home_file = tempfile::NamedTempFile::new().expect("temporary HOME file");
    let output = Command::new(env!("CARGO_BIN_EXE_rssh-cli"))
        .args(["completions", "bash"])
        .env("HOME", home_file.path())
        .output()
        .expect("run rssh CLI");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("profile"));
}

#[test]
fn unsupported_completion_shell_is_rejected_by_clap() {
    let output = rssh(&["completions", "tcsh"]);

    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("invalid value 'tcsh'"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn profile_named_fwd_is_opened_as_a_profile() {
    let home = tempfile::tempdir().expect("temporary HOME");
    let output = Command::new(env!("CARGO_BIN_EXE_rssh-cli"))
        .args(["profile", "open", "fwd"])
        .env("HOME", home.path())
        .env("RSSH_APP", "1")
        .output()
        .expect("run rssh CLI");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"\x1b]7337;open:fwd\x07");
}

#[test]
fn forward_open_uses_the_forward_osc_action() {
    let home = tempfile::tempdir().expect("temporary HOME");
    let output = Command::new(env!("CARGO_BIN_EXE_rssh-cli"))
        .args(["forward", "open", "tunnel"])
        .env("HOME", home.path())
        .env("RSSH_APP", "1")
        .output()
        .expect("run rssh CLI");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"\x1b]7337;fwd:tunnel\x07");
}

#[test]
fn profile_list_treats_cred_as_a_search_query() {
    let home = tempfile::tempdir().expect("temporary HOME");
    let output = Command::new(env!("CARGO_BIN_EXE_rssh-cli"))
        .args(["profile", "list", "cred"])
        .env("HOME", home.path())
        .output()
        .expect("run rssh CLI");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "No profiles.\n");
}

#[test]
fn bare_rssh_still_lists_profiles_outside_the_linux_gui_shadow() {
    let home = tempfile::tempdir().expect("temporary HOME");
    let output = Command::new(env!("CARGO_BIN_EXE_rssh-cli"))
        .env("HOME", home.path())
        .env("RSSH_APP", "1")
        .output()
        .expect("run rssh CLI");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "No profiles.\n");
}

#[test]
fn group_list_reports_an_empty_store() {
    let home = tempfile::tempdir().expect("temporary HOME");
    let output = Command::new(env!("CARGO_BIN_EXE_rssh-cli"))
        .args(["group", "list"])
        .env("HOME", home.path())
        .output()
        .expect("run rssh CLI");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "No groups.\n");
}

#[test]
fn group_add_uses_prompt_defaults_and_is_listed() {
    let home = tempfile::tempdir().expect("temporary HOME");
    let add = rssh_in_home(home.path(), &["group", "add"], "ops\n\n\n");
    assert!(
        add.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&add.stderr)
    );

    let list = rssh_in_home(home.path(), &["group", "list"], "");
    assert!(list.status.success());
    let stdout = String::from_utf8(list.stdout).expect("list is UTF-8");
    assert!(stdout.contains("ops"), "{stdout}");
    assert!(stdout.contains("#4A6CF7"), "{stdout}");
    assert!(stdout.contains("0"), "{stdout}");
}

#[test]
fn group_add_rejects_invalid_sort_order() {
    let home = tempfile::tempdir().expect("temporary HOME");
    let add = rssh_in_home(home.path(), &["group", "add"], "ops\n\nnot-a-number\n");

    assert_eq!(add.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&add.stderr).contains("numeric_arg_invalid"),
        "stderr: {}",
        String::from_utf8_lossy(&add.stderr)
    );

    let list = rssh_in_home(home.path(), &["group", "list"], "");
    assert_eq!(String::from_utf8_lossy(&list.stdout), "No groups.\n");
}

#[test]
fn group_add_rejects_invalid_color_without_echoing_control_bytes() {
    let home = tempfile::tempdir().expect("temporary HOME");
    let malicious = "\x1b]52;c;payload\x07";
    let input = format!("ops\n{malicious}\n0\n");
    let add = rssh_in_home(home.path(), &["group", "add"], &input);

    assert_eq!(add.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&add.stderr);
    assert!(stderr.contains("group_color_invalid"), "stderr: {stderr}");
    assert!(
        !stderr.contains('\x1b'),
        "stderr echoed control bytes: {stderr:?}"
    );

    let list = rssh_in_home(home.path(), &["group", "list"], "");
    assert_eq!(String::from_utf8_lossy(&list.stdout), "No groups.\n");
}

#[test]
fn group_edit_updates_name_color_and_order() {
    let home = tempfile::tempdir().expect("temporary HOME");
    let add = rssh_in_home(home.path(), &["group", "add"], "ops\n\n\n");
    assert!(add.status.success());

    let edit = rssh_in_home(
        home.path(),
        &["group", "edit", "ops"],
        "platform\n#112233\n7\n",
    );
    assert!(
        edit.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&edit.stderr)
    );

    let list = rssh_in_home(home.path(), &["group", "list"], "");
    let stdout = String::from_utf8(list.stdout).expect("list is UTF-8");
    assert!(stdout.contains("platform"), "{stdout}");
    assert!(stdout.contains("#112233"), "{stdout}");
    assert!(stdout.contains("7"), "{stdout}");
    assert!(!stdout.contains("ops"), "{stdout}");
}

#[test]
fn group_edit_rejects_invalid_sort_order() {
    let home = tempfile::tempdir().expect("temporary HOME");
    let add = rssh_in_home(home.path(), &["group", "add"], "ops\n\n\n");
    assert!(add.status.success());

    let edit = rssh_in_home(
        home.path(),
        &["group", "edit", "ops"],
        "platform\n#112233\nnot-a-number\n",
    );

    assert_eq!(edit.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&edit.stderr).contains("numeric_arg_invalid"),
        "stderr: {}",
        String::from_utf8_lossy(&edit.stderr)
    );

    let list = rssh_in_home(home.path(), &["group", "list"], "");
    let stdout = String::from_utf8(list.stdout).expect("list is UTF-8");
    assert!(stdout.contains("ops"), "{stdout}");
    assert!(!stdout.contains("platform"), "{stdout}");
}

#[test]
fn group_rm_removes_the_named_group() {
    let home = tempfile::tempdir().expect("temporary HOME");
    let add = rssh_in_home(home.path(), &["group", "add"], "ops\n\n\n");
    assert!(add.status.success());

    let remove = rssh_in_home(home.path(), &["group", "rm", "ops"], "y\n");
    assert!(
        remove.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&remove.stderr)
    );

    let list = rssh_in_home(home.path(), &["group", "list"], "");
    assert_eq!(String::from_utf8_lossy(&list.stdout), "No groups.\n");
}
