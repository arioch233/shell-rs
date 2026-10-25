//! 白名单节点发现和独立采集，不将远端输出拼接为 shell 代码

pub(crate) mod dvpp;
pub(crate) mod mpp;
mod rockchip;
pub(crate) mod tegra;
pub(crate) use rockchip::parse_nodes;

use super::snapshot::{CollectorDomain, MetricStatus};
use crate::terminal::shell_script;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default)]
pub(crate) struct DeviceCapabilities {
    pub soc: Option<String>,
    pub board_model: Option<String>,
    pub tegra: bool,
    pub ascend: bool,
    pub nodes: BTreeMap<String, MetricStatus>,
}

impl DeviceCapabilities {
    /// 祖先目录无权限不能证明设备存在，专用域需要平台或可读节点作为依据
    pub fn supports(&self, requested: CollectorDomain) -> bool {
        match requested {
            CollectorDomain::Rga | CollectorDomain::Npu => {
                self.nodes.iter().any(|(path, status)| {
                    domain(path) == Some(requested)
                        && *status != MetricStatus::Unsupported
                        && (self.soc.is_some() || *status == MetricStatus::Ready)
                })
            }
            CollectorDomain::Mpp => self.has_mpp(),
            CollectorDomain::Dvpp => self.ascend,
            CollectorDomain::Tegra => self.tegra,
            _ => true,
        }
    }

    pub fn has_mpp(&self) -> bool {
        self.nodes.iter().any(|(path, status)| {
            path.starts_with("/proc/mpp_service/") && *status != MetricStatus::Unsupported
        })
    }
}

pub(crate) struct NodeOutput {
    pub path: String,
    pub status: MetricStatus,
    pub body: String,
}

pub(super) const SUDO_NODES: &[&str] = &[
    "/sys/kernel/debug/rkrga/load",
    "/sys/kernel/debug/rkrga/hardware",
    "/sys/kernel/debug/rknpu/load",
    "/sys/kernel/debug/rknpu/driver_version",
    "/sys/kernel/debug/rknpu/version",
    mpp::CLOCK_SOURCE,
    tegra::ENGINES[0].usage,
    tegra::ENGINES[1].usage,
    tegra::ENGINES[2].usage,
    tegra::ENGINES[3].usage,
    tegra::ENGINES[4].usage,
    tegra::ENGINES[5].usage,
];

const FIXED: &[&str] = &[
    "/proc/device-tree/compatible",
    "/proc/device-tree/model",
    SUDO_NODES[0],
    SUDO_NODES[1],
    SUDO_NODES[2],
    SUDO_NODES[3],
    SUDO_NODES[4],
    SUDO_NODES[5],
    "/proc/mpp_service/supports-device",
    "/proc/mpp_service/timing_en",
    "/proc/mpp_service/rkvdec-core0/task_count",
    "/proc/mpp_service/rkvdec-core1/task_count",
    "/proc/mpp_service/rkvenc-core0/sessions-info",
    "/proc/mpp_service/rkvenc-core1/sessions-info",
    "/proc/mpp_service/version",
    "/proc/mpp_service/load",
    "/proc/mpp_service/load_interval",
    "/proc/mpp_service/sessions-summary",
    "/proc/mpp_service/sessions_summary",
    "/proc/pressure/cpu",
    "/proc/pressure/memory",
    "/proc/pressure/io",
];

fn allowed(path: &str) -> bool {
    if !path
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"/._-".contains(&b))
        || path.split('/').any(|part| part == "..")
    {
        return false;
    }
    if FIXED.contains(&path) || dvpp::SOURCES.contains(&path) || tegra::identify(path).is_some() {
        return true;
    }
    let parts: Vec<_> = path.split('/').collect();
    if parts.len() == 5 && parts[..3] == ["", "proc", "mpp_service"] {
        return [
            ("rkvdec-core", "task_count"),
            ("rkvenc-core", "sessions-info"),
        ]
        .iter()
        .any(|(prefix, field)| {
            parts[4] == *field
                && parts[3].strip_prefix(prefix).is_some_and(|id| {
                    !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit())
                })
        });
    }
    if parts.len() == 7
        && parts[..4] == ["", "sys", "class", "devfreq"]
        && !parts[4].is_empty()
        && parts[5..] == ["device", "gpuinfo"]
    {
        return true;
    }
    if parts.len() != 6 || parts[..3] != ["", "sys", "class"] {
        return false;
    }
    match parts[3] {
        "devfreq" => ["name", "load", "cur_freq"].contains(&parts[5]),
        "thermal" => parts[4].starts_with("thermal_zone") && ["type", "temp"].contains(&parts[5]),
        _ => false,
    }
}

/// 先检查祖先目录的遍历权限，不能把 EACCES 当作 ENOENT
const PROBE: &str = r#"
check() {
 p="$1"; d="${p%/*}"; state=ready
 while test -n "$d"; do
   if test -d "$d" && ! test -x "$d"; then state=denied; break; fi
   d="${d%/*}"
 done
 if test "$state" = ready; then
   if ! test -e "$p"; then state=unsupported
   elif ! test -r "$p"; then state=denied; fi
 fi
}
"#;

pub(crate) fn probe_command() -> String {
    let fixed = FIXED
        .iter()
        .chain(dvpp::SOURCES)
        .map(|path| (*path).to_owned())
        .chain(tegra::paths())
        .collect::<Vec<_>>()
        .join(" ");
    shell_script(&format!(
        r#"export LC_ALL=C
{PROBE}
seen=" "
for p in {fixed} /proc/mpp_service/rkvdec-core*/task_count /proc/mpp_service/rkvenc-core*/sessions-info /sys/class/devfreq/*/name /sys/class/devfreq/*/load /sys/class/devfreq/*/cur_freq /sys/class/devfreq/*/device/gpuinfo /sys/class/thermal/thermal_zone*/type /sys/class/thermal/thermal_zone*/temp; do
 case "$p" in *\**) continue;; esac
 case "$seen" in *" $p "*) continue;; esac
 seen="$seen$p "
 check "$p"
 printf '@@node %s %s\n' "$p" "$state"
 if test "$state" = ready; then
  case "$p" in /proc/device-tree/*) tr '\000' '\n' < "$p";; esac
 fi
 echo @@endnode
done
echo @@complete
"#
    ))
}

pub(crate) fn nodes(output: &str) -> Result<Vec<NodeOutput>, MetricStatus> {
    if output.lines().last() != Some("@@complete") {
        return Err(MetricStatus::Failed("incomplete_output".into()));
    }
    let mut result = Vec::new();
    let mut current: Option<NodeOutput> = None;
    for line in output.lines() {
        if let Some(header) = line.strip_prefix("@@node ") {
            if current.is_some() {
                return Err(MetricStatus::Failed("invalid_frame".into()));
            }
            let Some((path, state)) = header.rsplit_once(' ') else {
                return Err(MetricStatus::Failed("invalid_frame".into()));
            };
            if !allowed(path) {
                return Err(MetricStatus::Failed("invalid_source".into()));
            }
            let status = match state {
                "ready" => MetricStatus::Ready,
                "denied" => MetricStatus::PermissionDenied,
                "unsupported" => MetricStatus::Unsupported,
                "sudo_denied" => MetricStatus::SudoDenied,
                "sudo_unavailable" => MetricStatus::SudoUnavailable,
                "timeout" => MetricStatus::Timeout,
                "failed" => MetricStatus::Failed("read_failed".into()),
                _ => return Err(MetricStatus::Failed("invalid_status".into())),
            };
            current = Some(NodeOutput {
                path: path.into(),
                status,
                body: String::new(),
            });
        } else if line == "@@endnode" {
            let node = current
                .take()
                .ok_or_else(|| MetricStatus::Failed("invalid_frame".into()))?;
            if result.iter().any(|old: &NodeOutput| old.path == node.path) {
                return Err(MetricStatus::Failed("duplicate_source".into()));
            }
            result.push(node);
        } else if let Some(code) = line.strip_prefix("@@status ") {
            if let Some(node) = &mut current {
                node.status = match code {
                    "0" => node.status.clone(),
                    "124" | "137" => MetricStatus::Timeout,
                    _ => MetricStatus::Failed("read_failed".into()),
                };
            }
        } else if let Some(node) = &mut current {
            node.body.push_str(line);
            node.body.push('\n');
        }
    }
    if current.is_some() {
        return Err(MetricStatus::Failed("incomplete_output".into()));
    }
    Ok(result)
}

pub(crate) fn parse_probe(output: &str) -> Result<DeviceCapabilities, MetricStatus> {
    let mut caps = DeviceCapabilities::default();
    for node in nodes(output)? {
        if node.path == "/proc/device-tree/compatible" {
            caps.soc = node.body.lines().find_map(|line| {
                line.strip_prefix("rockchip,")
                    .filter(|name| name.bytes().all(|b| b.is_ascii_alphanumeric()))
                    .map(str::to_owned)
            });
        }
        if node.path == "/proc/device-tree/compatible" && node.status == MetricStatus::Ready {
            caps.ascend = node
                .body
                .lines()
                .any(|line| line.starts_with("hisilicon,ascend"));
            caps.tegra = node
                .body
                .lines()
                .any(|line| line.starts_with("nvidia,tegra"));
        }
        if node.path == "/proc/device-tree/model" && node.status == MetricStatus::Ready {
            caps.board_model = node
                .body
                .lines()
                .find(|line| !line.trim().is_empty())
                .map(str::to_owned);
        }
        caps.nodes.insert(node.path, node.status);
    }
    Ok(caps)
}

pub(crate) fn domain(path: &str) -> Option<CollectorDomain> {
    Some(if path.contains("/rkrga/") {
        CollectorDomain::Rga
    } else if path.contains("/rknpu/") {
        CollectorDomain::Npu
    } else if path.contains("/mpp_service/") || path == mpp::CLOCK_SOURCE {
        CollectorDomain::Mpp
    } else if dvpp::SOURCES.contains(&path) {
        CollectorDomain::Dvpp
    } else if tegra::identify(path).is_some() {
        CollectorDomain::Tegra
    } else if path.contains("/devfreq/") {
        if path.contains("npu") {
            CollectorDomain::Npu
        } else {
            CollectorDomain::Devfreq
        }
    } else if path.contains("/thermal/") {
        CollectorDomain::Thermal
    } else if path.contains("/pressure/") {
        CollectorDomain::Psi
    } else {
        return None;
    })
}

pub(crate) fn command(
    caps: &DeviceCapabilities,
    requested: CollectorDomain,
    allow_sudo: bool,
) -> Option<String> {
    if !caps.supports(requested) {
        return None;
    }
    let allow_sudo = allow_sudo
        && matches!(
            requested,
            CollectorDomain::Rga
                | CollectorDomain::Npu
                | CollectorDomain::Mpp
                | CollectorDomain::Tegra
        );
    let paths = caps
        .nodes
        .iter()
        .filter(|(path, state)| {
            allowed(path)
                && tegra::identify(path).is_none_or(|(engine, _)| tegra::present(caps, engine))
                && (path.as_str() != mpp::CLOCK_SOURCE || caps.has_mpp())
                && domain(path) == Some(requested)
                && (**state == MetricStatus::Ready
                    || (allow_sudo
                        && **state == MetricStatus::PermissionDenied
                        && SUDO_NODES.contains(&path.as_str())))
        })
        .map(|(path, _)| path.as_str())
        .collect::<Vec<_>>();
    if paths.is_empty() {
        return None;
    }
    // sudo 仅作用于固定读取命令，外层 shell、timeout 与其他节点保持原身份
    let privileged = if allow_sudo {
        sudo_reader()
    } else {
        String::new()
    };
    let fallback = if allow_sudo {
        "if test \"$state\" = denied; then sudo_read \"$p\"; fi"
    } else {
        ""
    };
    Some(shell_script(&format!(
        r#"export LC_ALL=C
{PROBE}
{privileged}
for p in {}; do
 check "$p"
 elevated=0
 s=0
 {fallback}
 printf '@@node %s %s\n' "$p" "$state"
 if test "$state" = ready; then
   if test "$elevated" = 1; then printf '%s\n' "$body"; s=0
   elif command -v timeout >/dev/null 2>&1; then timeout -s KILL 3 head -c 262145 "$p"; s=$?
   else head -c 262145 "$p"; s=$?; fi
   printf '\n@@status %s\n' "$s"
 fi
 echo @@endnode
 if test "$state" = timeout || test "$s" = 124 || test "$s" = 137; then break; fi
done
echo @@complete
"#,
        paths.join(" ")
    )))
}

fn sudo_reader() -> String {
    format!(
        r#"
sudo_read() {{
 case "$1" in {}) ;; *) return;; esac
 elevated=1
 if ! command -v sudo >/dev/null 2>&1 || ! test -x /usr/bin/head; then state=sudo_unavailable; return; fi
 body="$(if command -v timeout >/dev/null 2>&1; then timeout -s KILL 3 sudo -n /usr/bin/head -c 262145 "$1"; else sudo -n /usr/bin/head -c 262145 "$1"; fi 2>&1)"
 code=$?
 case "$code" in
  0) state=ready;;
  124|137) state=timeout;;
  *) case "$body" in
      *sudo:*|*"not allowed to execute"*|*"not in the sudoers"*) state=sudo_denied;;
      *"No such file or directory"*) state=unsupported;;
      *"Permission denied"*) state=denied;;
      *) state=failed;;
     esac;;
 esac
}}
"#,
        SUDO_NODES.join("|")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inaccessible_debugfs_does_not_invent_vendor_hardware() {
        for compatible in ["nvidia,tegra234", "hisilicon,ascend610", "generic,linux"] {
            let output = format!(
                "@@node /proc/device-tree/compatible ready\n{compatible}\n@@endnode\n@@node /sys/kernel/debug/rkrga/load denied\n@@endnode\n@@node /sys/kernel/debug/rknpu/load denied\n@@endnode\n@@node /sys/kernel/debug/clk/clk_summary denied\n@@endnode\n@@complete"
            );
            let caps = parse_probe(&output).unwrap();
            for domain in [
                CollectorDomain::Rga,
                CollectorDomain::Npu,
                CollectorDomain::Mpp,
            ] {
                assert!(!caps.supports(domain));
                assert!(command(&caps, domain, false).is_none());
                assert!(command(&caps, domain, true).is_none());
            }
            // 保留原始权限状态供诊断，不能篡改成已经证实的节点缺失
            assert_eq!(caps.nodes[SUDO_NODES[0]], MetricStatus::PermissionDenied);
        }
        let mut caps = DeviceCapabilities {
            soc: Some("rk3588".into()),
            ..Default::default()
        };
        caps.nodes
            .insert(SUDO_NODES[0].into(), MetricStatus::PermissionDenied);
        assert!(caps.supports(CollectorDomain::Rga));
        assert!(command(&caps, CollectorDomain::Rga, true).is_some());
        assert!(!caps.supports(CollectorDomain::Npu));
        caps.nodes
            .insert(SUDO_NODES[0].into(), MetricStatus::Unsupported);
        assert!(!caps.supports(CollectorDomain::Rga));
        caps.soc = None;
        caps.nodes.insert(SUDO_NODES[2].into(), MetricStatus::Ready);
        assert!(caps.supports(CollectorDomain::Npu));
    }

    #[test]
    fn sudo_only_adds_denied_nodes_from_the_fixed_allowlist() {
        let mut caps = DeviceCapabilities {
            soc: Some("rk3588".into()),
            ..Default::default()
        };
        for path in SUDO_NODES.iter().chain(
            [
                "/proc/mpp_service/load",
                "/sys/class/devfreq/gpu/load",
                "/sys/kernel/debug/rknpu/evil",
            ]
            .iter(),
        ) {
            caps.nodes
                .insert((*path).into(), MetricStatus::PermissionDenied);
        }
        assert!(command(&caps, CollectorDomain::Rga, false).is_none());
        assert!(command(&caps, CollectorDomain::Npu, false).is_none());
        assert!(command(&caps, CollectorDomain::Rga, true).is_some());
        assert!(command(&caps, CollectorDomain::Npu, true).is_some());
        assert!(command(&caps, CollectorDomain::Mpp, true).is_some());
        assert!(!SUDO_NODES.contains(&"/proc/mpp_service/load"));
        assert!(command(&caps, CollectorDomain::Devfreq, true).is_none());
        caps.nodes
            .insert(SUDO_NODES[0].into(), MetricStatus::Unsupported);
        caps.nodes.remove(SUDO_NODES[1]);
        assert!(command(&caps, CollectorDomain::Rga, true).is_none());
    }

    #[test]
    fn generic_clock_tree_alone_does_not_claim_mpp_hardware() {
        let mut caps = DeviceCapabilities::default();
        caps.nodes
            .insert(mpp::CLOCK_SOURCE.into(), MetricStatus::Ready);
        caps.nodes.insert(
            "/proc/mpp_service/version".into(),
            MetricStatus::Unsupported,
        );
        assert!(command(&caps, CollectorDomain::Mpp, false).is_none());
        caps.nodes
            .insert("/proc/mpp_service/version".into(), MetricStatus::Ready);
        assert!(command(&caps, CollectorDomain::Mpp, false).is_some());
    }

    #[test]
    fn npu_version_is_probed_and_read_with_the_host_permission_choice() {
        let path = "/sys/kernel/debug/rknpu/version";
        assert!(FIXED.contains(&path));
        let caps = parse_probe(&format!("@@node /proc/device-tree/compatible ready\nrockchip,rk3588\n@@endnode\n@@node {path} denied\n@@endnode\n@@complete")).unwrap();
        assert!(command(&caps, CollectorDomain::Npu, false).is_none());
        assert!(command(&caps, CollectorDomain::Npu, true).is_some());
        assert!(SUDO_NODES.iter().all(|path| allowed(path)));
    }

    #[cfg(unix)]
    #[test]
    fn sudo_reader_handles_authorization_absence_timeout_and_literal_output() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("sudo");
        std::fs::write(&executable, r#"#!/bin/sh
if test "$#" != 5 || test "$1" != -n || test "$2" != /usr/bin/head || test "$3" != -c || test "$4" != 262145; then exit 99; fi
printf '%s' "$SHELLRS_TEST_BODY"
exit "$SHELLRS_TEST_CODE"
"#).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        let run = |path: &str, code: &str, body: &str| {
            let script = format!(
                "{}\nstate=denied; elevated=0; sudo_read \"$SHELLRS_TEST_PATH\"; printf '%s\\n' \"$state\"; if test \"$state\" = ready; then printf '%s' \"$body\"; fi",
                sudo_reader()
            );
            let output = std::process::Command::new("/bin/sh")
                .args(["-c", &script])
                .env("PATH", directory.path())
                .env("SHELLRS_TEST_PATH", path)
                .env("SHELLRS_TEST_CODE", code)
                .env("SHELLRS_TEST_BODY", body)
                .output()
                .unwrap();
            assert!(output.status.success());
            String::from_utf8(output.stdout).unwrap()
        };
        assert_eq!(
            run(
                SUDO_NODES[0],
                "0",
                "scheduler[0]: rga3\n load = 2%\n$(false)"
            ),
            "ready\nscheduler[0]: rga3\n load = 2%\n$(false)"
        );
        assert_eq!(
            run(SUDO_NODES[2], "1", "sudo: a password is required"),
            "sudo_denied\n"
        );
        assert_eq!(
            run(
                "/sys/kernel/debug/rknpu/version",
                "0",
                "RKNPU driver: v0.9.8"
            ),
            "ready\nRKNPU driver: v0.9.8"
        );
        assert_eq!(
            run(SUDO_NODES[2], "1", "sudo: not allowed to execute"),
            "sudo_denied\n"
        );
        assert_eq!(
            run(
                SUDO_NODES[2],
                "1",
                "Sorry, user rockchip is not allowed to execute this command"
            ),
            "sudo_denied\n"
        );
        assert_eq!(
            run(SUDO_NODES[3], "1", "head: No such file or directory"),
            "unsupported\n"
        );
        assert_eq!(
            run(SUDO_NODES[2], "1", "head: Permission denied"),
            "denied\n"
        );
        assert_eq!(run(SUDO_NODES[0], "137", "Killed"), "timeout\n");
        assert_eq!(run("/etc/shadow", "0", "must not be read"), "denied\n");
        std::fs::remove_file(executable).unwrap();
        assert_eq!(run(SUDO_NODES[0], "0", ""), "sudo_unavailable\n");
    }

    #[test]
    fn sources_cannot_escape_the_read_only_allowlist() {
        for path in [
            "/etc/shadow",
            "/sys/class/devfreq/../../etc/shadow",
            "/sys/class/devfreq/a;id/load",
            "/proc/mpp_service/evil",
            "/proc/mpp_service/rkvdec-core0/reset",
            "/proc/mpp_service/rkvdec-core0/../timing_en",
            "/proc/mpp_service/rkvdec-corex/task_count",
            "/sys/kernel/debug/clk/clk_ignore_unused",
            "/sys/class/devfreq/fb000000.gpu/device/reset",
            "/sys/class/devfreq/fb000000.gpu/device/../gpuinfo",
        ] {
            assert!(!allowed(path));
        }
        assert!(allowed("/sys/class/devfreq/fdab0000.npu/load"));
        assert!(allowed("/proc/mpp_service/rkvdec-core7/task_count"));
        assert!(allowed("/proc/mpp_service/rkvenc-core7/sessions-info"));
        assert!(allowed(mpp::CLOCK_SOURCE));
        assert!(allowed("/sys/class/devfreq/fb000000.gpu/device/gpuinfo"));
        assert!(nodes("@@node /etc/shadow ready\n@@endnode\n@@complete").is_err());
        assert!(nodes("@@node /proc/mpp_service/load ready\n").is_err());
    }
    #[test]
    #[ignore = "需要显式指定 SHELLRS_MONITOR_TEST_HOST，只读 SSH 实机验证"]
    fn rockchip_read_only_smoke() {
        let host = std::env::var("SHELLRS_MONITOR_TEST_HOST")
            .expect("SHELLRS_MONITOR_TEST_HOST is required");
        let run = |command: String| {
            let output = std::process::Command::new("ssh")
                .args([
                    "-o",
                    "BatchMode=yes",
                    "-o",
                    "ConnectTimeout=8",
                    "-o",
                    "StrictHostKeyChecking=yes",
                    "-o",
                    "ServerAliveInterval=5",
                    "-o",
                    "ServerAliveCountMax=2",
                    "--",
                    &host,
                    &command,
                ])
                .output()
                .expect("SSH starts");
            assert!(
                output.status.success(),
                "SSH read failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(output.stdout.len() <= 256 * 1024, "output limit exceeded");
            String::from_utf8(output.stdout).unwrap()
        };
        let caps = parse_probe(&run(probe_command())).unwrap();
        assert_eq!(caps.soc.as_deref(), Some("rk3588"));
        let allow_sudo = std::env::var("SHELLRS_MONITOR_TEST_SUDO").as_deref() == Ok("1");
        for domain in [
            CollectorDomain::Rga,
            CollectorDomain::Npu,
            CollectorDomain::Mpp,
            CollectorDomain::Devfreq,
            CollectorDomain::Thermal,
            CollectorDomain::Psi,
        ] {
            let output = command(&caps, domain, allow_sudo)
                .map(&run)
                .unwrap_or_else(|| "@@complete".into());
            let rows = parse_nodes(&output, &caps, domain).unwrap();
            assert!(
                !rows.iter().any(|row| matches!(
                    row.status,
                    MetricStatus::Failed(_) | MetricStatus::Timeout
                )),
                "invalid {} output",
                domain.key()
            );
            if allow_sudo && matches!(domain, CollectorDomain::Rga | CollectorDomain::Npu) {
                let prefix = if domain == CollectorDomain::Rga {
                    "rockchip/rga/scheduler/"
                } else {
                    "rockchip/npu/core/"
                };
                assert!(
                    rows.iter().any(|row| row.id.starts_with(prefix)
                        && row.value.is_some()
                        && row.status == MetricStatus::Ready),
                    "no privileged {} metrics",
                    domain.key()
                );
            }
            eprintln!("{}: {} metrics", domain.key(), rows.len());
        }
    }
}
