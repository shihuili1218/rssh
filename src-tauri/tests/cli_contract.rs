#![cfg(feature = "cli")]

use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use rssh_lib::db::Db;
use rssh_lib::models::{
    Credential, CredentialType, Forward, ForwardType, Group, Profile, SshAlgorithms,
};

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

fn assert_dynamic_completion_registration(shell: &str) {
    let home = tempfile::tempdir().expect("temporary HOME");
    let output = rssh_in_home(home.path(), &["completions", shell], "");
    assert!(
        output.status.success(),
        "{shell} stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("completion script is UTF-8");
    assert!(
        stdout.contains("_RSSH_COMPLETE"),
        "{shell} completion does not call the runtime completer:\n{stdout}"
    );
    assert!(
        stdout.contains("rssh") && stdout.contains("--"),
        "{shell} completion does not pass command words back to rssh:\n{stdout}"
    );
}

fn complete_in_home(home: &Path, words: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rssh-cli"))
        .arg("--")
        .args(words)
        .env("HOME", home)
        .env("_RSSH_COMPLETE", "fish")
        .output()
        .expect("run dynamic completion")
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
    assert_dynamic_completion_registration("bash");
}

#[test]
fn zsh_completions_follow_the_typed_command_tree() {
    assert_dynamic_completion_registration("zsh");
}

#[test]
fn fish_completions_follow_the_typed_command_tree() {
    assert_dynamic_completion_registration("fish");
}

#[test]
fn powershell_completions_follow_the_typed_command_tree() {
    assert_dynamic_completion_registration("powershell");
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
    assert!(String::from_utf8_lossy(&output.stdout).contains("_RSSH_COMPLETE"));
}

#[test]
fn runtime_completion_reads_named_resources() {
    let home = tempfile::tempdir().expect("temporary HOME");
    let db = Db::open(&home.path().join(".rssh")).expect("open test database");
    let credential = Credential {
        id: "credential-id".into(),
        name: "Credential Deploy".into(),
        username: "deploy".into(),
        credential_type: CredentialType::None,
        secret: None,
        save_to_remote: false,
    };
    rssh_lib::db::credential::insert(&db, &credential).expect("insert credential");
    let group = Group {
        id: "group-id".into(),
        name: "Group Platform".into(),
        color: "#112233".into(),
        sort_order: 0,
    };
    rssh_lib::db::group::insert(&db, &group).expect("insert group");
    let profile = Profile {
        id: "profile-id".into(),
        name: "Profile Production".into(),
        host: "production.example.com".into(),
        port: 22,
        credential_id: credential.id,
        bastion_profile_id: None,
        init_command: None,
        group_id: Some(group.id),
        algorithms: SshAlgorithms::default(),
    };
    rssh_lib::db::profile::insert(&db, &profile).expect("insert profile");
    let forward = Forward {
        id: "forward-id".into(),
        name: "Forward Database".into(),
        forward_type: ForwardType::Local,
        local_port: 5432,
        remote_host: "database.internal".into(),
        remote_port: 5432,
        profile_id: profile.id,
        group_id: None,
    };
    rssh_lib::db::forward::insert(&db, &forward).expect("insert forward");
    drop(db);

    for (words, expected) in [
        (
            &["rssh", "profile", "open", "pro"][..],
            "Profile Production",
        ),
        (
            &["rssh", "credential", "edit", "cre"][..],
            "Credential Deploy",
        ),
        (&["rssh", "forward", "open", "for"][..], "Forward Database"),
        (&["rssh", "group", "rm", "gro"][..], "Group Platform"),
    ] {
        let output = complete_in_home(home.path(), words);
        assert!(
            output.status.success(),
            "completion stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .any(|candidate| candidate == expected),
            "missing {expected:?} in completion output: {}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
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
