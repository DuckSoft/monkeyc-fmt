use std::fs;
use std::io::Write;
use std::process::{Command, Output, Stdio};
use tempfile::tempdir;

fn command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_monkeyc-fmt"))
}

fn run_with_stdin(args: &[&str], input: &str) -> Output {
    let mut child = command()
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("formatter starts");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn text(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).expect("CLI diagnostics are UTF-8")
}

#[test]
fn stdin_to_stdout_works_for_implicit_and_explicit_stdin() {
    for args in [&[][..], &["-"][..]] {
        let output = run_with_stdin(args, "var x=1;");
        assert!(output.status.success(), "{}", text(&output.stderr));
        assert_eq!(text(&output.stdout), "var x = 1;\n");
        assert!(output.stderr.is_empty());
    }
}

#[test]
fn a_single_file_is_formatted_to_stdout_without_mutation() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("input.mc");
    fs::write(&path, "function f(){return 1;}").unwrap();

    let output = command().arg(&path).output().unwrap();
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), "function f() {\n    return 1;\n}\n");
    assert_eq!(fs::read_to_string(path).unwrap(), "function f(){return 1;}");
}

#[test]
fn check_distinguishes_clean_changed_and_erroneous_inputs_without_mutating() {
    let dir = tempdir().unwrap();
    let clean = dir.path().join("clean.mc");
    let dirty = dir.path().join("dirty.mc");
    let invalid = dir.path().join("invalid.mc");
    fs::write(&clean, "var x = 1;\n").unwrap();
    fs::write(&dirty, "var y=2;").unwrap();
    fs::write(&invalid, "function broken( {").unwrap();

    let clean_output = command().args(["--check"]).arg(&clean).output().unwrap();
    assert_eq!(clean_output.status.code(), Some(0));

    let dirty_output = command().args(["--check"]).arg(&dirty).output().unwrap();
    assert_eq!(dirty_output.status.code(), Some(1));
    assert_eq!(fs::read_to_string(&dirty).unwrap(), "var y=2;");

    let invalid_output = command().args(["--check"]).arg(&invalid).output().unwrap();
    assert_eq!(invalid_output.status.code(), Some(2));
    assert_eq!(fs::read_to_string(&invalid).unwrap(), "function broken( {");
}

#[test]
fn check_accepts_multiple_paths_and_reports_if_any_need_changes() {
    let dir = tempdir().unwrap();
    let first = dir.path().join("first.mc");
    let second = dir.path().join("second.mc");
    fs::write(&first, "var first = 1;\n").unwrap();
    fs::write(&second, "var second=2;").unwrap();

    let output = command()
        .arg("--check")
        .arg(&first)
        .arg(&second)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(fs::read_to_string(&first).unwrap(), "var first = 1;\n");
    assert_eq!(fs::read_to_string(&second).unwrap(), "var second=2;");

    fs::write(&second, "var second = 2;\n").unwrap();
    let output = command()
        .arg("--check")
        .arg(&first)
        .arg(&second)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
}

#[test]
fn write_updates_multiple_files_but_never_stdout() {
    let dir = tempdir().unwrap();
    let first = dir.path().join("first.mc");
    let second = dir.path().join("second.mc");
    fs::write(&first, "var first=1;").unwrap();
    fs::write(&second, "var second=2;").unwrap();

    let output = command()
        .arg("--write")
        .arg(&first)
        .arg(&second)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(output.stdout.is_empty());
    assert_eq!(fs::read_to_string(first).unwrap(), "var first = 1;\n");
    assert_eq!(fs::read_to_string(second).unwrap(), "var second = 2;\n");
}

#[test]
fn write_preserves_a_file_when_formatting_fails() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("broken.mc");
    let original = b"function broken( {\n";
    fs::write(&path, original).unwrap();

    let output = command().arg("--write").arg(&path).output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(fs::read(path).unwrap(), original);
}

#[test]
fn conflicting_modes_and_write_with_stdin_are_usage_errors() {
    let conflict = command().args(["--check", "--write"]).output().unwrap();
    assert_eq!(conflict.status.code(), Some(2));

    let implicit = run_with_stdin(&["--write"], "var x=1;");
    assert_eq!(implicit.status.code(), Some(2));

    let explicit = run_with_stdin(&["--write", "-"], "var x=1;");
    assert_eq!(explicit.status.code(), Some(2));
}

#[test]
fn plain_stdout_mode_rejects_multiple_inputs() {
    let dir = tempdir().unwrap();
    let first = dir.path().join("first.mc");
    let second = dir.path().join("second.mc");
    fs::write(&first, "var first = 1;\n").unwrap();
    fs::write(&second, "var second = 2;\n").unwrap();

    let output = command().arg(first).arg(second).output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
}

#[cfg(unix)]
#[test]
fn write_preserves_permissions_and_rejects_symlinks() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    let dir = tempdir().unwrap();
    let path = dir.path().join("mode.mc");
    fs::write(&path, "var x=1;").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();

    let output = command().arg("--write").arg(&path).output().unwrap();
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );

    let target = dir.path().join("target.mc");
    let link = dir.path().join("link.mc");
    fs::write(&target, "var target=1;").unwrap();
    symlink(&target, &link).unwrap();
    let before = fs::read(&target).unwrap();

    let output = command().arg("--write").arg(&link).output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(fs::read(target).unwrap(), before);
}

#[test]
fn invalid_utf8_is_rejected_without_overwrite() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("invalid-utf8.mc");
    let bytes = b"var x = \xff;\n";
    fs::write(&path, bytes).unwrap();

    let output = command().arg("--write").arg(&path).output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(fs::read(path).unwrap(), bytes);
}
