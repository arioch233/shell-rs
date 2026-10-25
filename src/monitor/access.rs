//! 只管理 ShellRS 自己的硬件只读 sudoers 规则

use super::collectors::SUDO_NODES;
use crate::terminal::{ExecOutput, shell_script, shell_script_with_input};

const READY: &str = "@@shellrs-monitor-access ready";

pub(super) fn command() -> String {
    authorization_command(&installer())
}

fn authorization_command(script: &str) -> String {
    let install = shell_script(script);
    shell_script_with_input(&format!(
        "if test \"$(id -u)\" = 0; then printf '%s\\n' '{READY}'; else sudo -S -k -p '' -- {install}; fi"
    ))
}

pub(super) fn installed(output: &ExecOutput) -> bool {
    // 成功只认脚本退出码，避免密码脱敏改写恰好与密码重叠的状态文本
    output.exit_code == Some(0)
}

fn installer() -> String {
    let commands = SUDO_NODES
        .iter()
        .map(|path| format!("/usr/bin/head -c 262145 {path}"))
        .collect::<Vec<_>>()
        .join(", ");
    include_str!("access_install.sh").replace("@COMMANDS@", &commands)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn fixture() -> tempfile::TempDir {
        use std::os::unix::fs::PermissionsExt as _;
        let directory = tempfile::tempdir().unwrap();
        let bin = directory.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        std::fs::create_dir(directory.path().join("sudoers.d")).unwrap();
        for (name, body) in [
            ("id", "printf '0\\n'"),
            ("stat", "case \"$2\" in %u) echo 0;; %a) echo 755;; esac"),
            ("chown", "exit 0"),
            (
                "visudo",
                r#"
if test "$1" = -cf; then
    test "$FAIL_STAGE" != candidate || exit 1
    exit 0
fi
target="$FIXTURE/sudoers.d/shellrs-monitor-1000"
if test -f "$target" && grep -q NOPASSWD "$target"; then
    test "$FAIL_STAGE" != after || exit 1
    test "$FAIL_STAGE" != excluded || exit 0
fi
printf '%s: parsed OK\n' "$target"
"#,
            ),
        ] {
            let file = bin.join(name);
            std::fs::write(&file, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        directory
    }

    #[cfg(unix)]
    fn run_fixture(directory: &std::path::Path, stage: &str, uid: &str) -> std::process::Output {
        let script = installer()
            .replace(
                "PATH=/usr/sbin:/usr/bin:/sbin:/bin",
                &format!(
                    "PATH={}/bin:/usr/sbin:/usr/bin:/sbin:/bin",
                    directory.display()
                ),
            )
            .replace(
                "dir=/etc/sudoers.d",
                &format!("dir={}/sudoers.d", directory.display()),
            );
        std::process::Command::new("sh")
            .arg("-c")
            .arg(shell_script(&script))
            .env("FIXTURE", directory)
            .env("FAIL_STAGE", stage)
            .env("SUDO_UID", uid)
            .output()
            .unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn installer_updates_only_managed_rules_and_rolls_back_failed_validation() {
        use std::os::unix::fs::PermissionsExt as _;
        for stage in ["", "candidate", "after", "excluded"] {
            for existing in [false, true] {
                let fixture = fixture();
                let dir = fixture.path().join("sudoers.d");
                let target = dir.join("shellrs-monitor-1000");
                let original = "# ShellRS managed monitor access v1\n# previous rule\n";
                if existing {
                    std::fs::write(&target, original).unwrap();
                }
                let output = run_fixture(fixture.path(), stage, "1000");
                assert_eq!(
                    output.status.success(),
                    stage.is_empty(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
                if stage.is_empty() {
                    let rule = std::fs::read_to_string(&target).unwrap();
                    assert_eq!(
                        rule.matches("/usr/bin/head -c 262145 ").count(),
                        SUDO_NODES.len()
                    );
                    assert!(rule.contains("#1000 ALL=(root) NOPASSWD:"));
                    assert_eq!(
                        std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
                        0o440
                    );
                    assert!(run_fixture(fixture.path(), "", "1000").status.success());
                } else if existing {
                    assert_eq!(std::fs::read_to_string(&target).unwrap(), original);
                } else {
                    assert!(!target.exists());
                }
                assert!(std::fs::read_dir(&dir).unwrap().all(|entry| {
                    !entry
                        .unwrap()
                        .file_name()
                        .to_string_lossy()
                        .starts_with('.')
                }));
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn installer_refuses_unmanaged_files_symlinks_and_invalid_users() {
        let fixture = fixture();
        let target = fixture.path().join("sudoers.d/shellrs-monitor-1000");
        std::fs::write(&target, "administrator's file").unwrap();
        assert!(!run_fixture(fixture.path(), "", "1000").status.success());
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "administrator's file"
        );
        std::fs::remove_file(&target).unwrap();
        let other = fixture.path().join("other");
        std::fs::write(&other, "untouched").unwrap();
        std::os::unix::fs::symlink(&other, &target).unwrap();
        assert!(!run_fixture(fixture.path(), "", "1000").status.success());
        assert_eq!(std::fs::read_to_string(other).unwrap(), "untouched");
        for uid in ["", "0", "1000 ALL=ALL", "$(id)", "../1000"] {
            assert!(!run_fixture(fixture.path(), "", uid).status.success());
        }
    }

    #[cfg(unix)]
    #[test]
    fn generated_numeric_user_rule_passes_visudo_when_available() {
        let probe = std::process::Command::new("/usr/sbin/visudo")
            .arg("-V")
            .output();
        if probe.is_err() {
            return;
        }
        let fixture = fixture();
        assert!(run_fixture(fixture.path(), "", "1000").status.success());
        let output = std::process::Command::new("/usr/sbin/visudo")
            .arg("-cf")
            .arg(fixture.path().join("sudoers.d/shellrs-monitor-1000"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[cfg(unix)]
    #[test]
    fn authorization_passes_password_on_stdin_without_shell_interpolation() {
        use std::{
            io::Write as _,
            os::unix::fs::PermissionsExt as _,
            process::{Command, Stdio},
        };
        let fixture = fixture();
        let bin = fixture.path().join("bin");
        std::fs::write(bin.join("id"), "#!/bin/sh\necho 1000\n").unwrap();
        let sudo = bin.join("sudo");
        std::fs::write(
            &sudo,
            r#"#!/bin/sh
IFS= read -r password
test "$password" = 'secret $() ! "' || exit 1
while test "$1" != --; do shift; done
shift
exec "$@"
"#,
        )
        .unwrap();
        std::fs::set_permissions(&sudo, std::fs::Permissions::from_mode(0o755)).unwrap();
        let command = authorization_command("printf '%s\\n' '@@shellrs-monitor-access ready'");
        assert!(!command.contains("secret"));
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(command)
            .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"secret $() ! \"\n")
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), READY);
    }

    #[test]
    fn completion_requires_successful_exit_even_when_status_text_is_redacted() {
        let mut output = ExecOutput {
            stdout: READY.into(),
            exit_code: Some(0),
            ..Default::default()
        };
        assert!(installed(&output));
        output.exit_code = Some(1);
        assert!(!installed(&output));
        output.exit_code = None;
        assert!(!installed(&output));
        output.exit_code = Some(0);
        output.stdout = "@@shellrs-monitor-access <redacted>".into();
        assert!(installed(&output));
    }
}
