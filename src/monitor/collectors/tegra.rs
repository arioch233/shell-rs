//! Orin NX 的固定媒体引擎与 ACTMON 只读接口

use super::{DeviceCapabilities, nodes};
use crate::monitor::{snapshot::MetricStatus, tegra::TegraTracker};
use std::time::Instant;

pub(crate) struct Engine {
    pub device: &'static str,
    pub name: &'static str,
    pub usage: &'static str,
}

pub(crate) const ENGINES: &[Engine] = &[
    Engine {
        device: "154c0000.nvenc",
        name: "NVENC",
        usage: "/sys/kernel/debug/tegra-host1x/actmon/nvenc/module0/usage",
    },
    Engine {
        device: "15480000.nvdec",
        name: "NVDEC",
        usage: "/sys/kernel/debug/tegra-host1x/actmon/nvdec/module0/usage",
    },
    Engine {
        device: "15380000.nvjpg",
        name: "NVJPG 0",
        usage: "/sys/kernel/debug/tegra-host1x/actmon/nvjpg/module0/usage",
    },
    Engine {
        device: "15540000.nvjpg",
        name: "NVJPG 1",
        usage: "/sys/kernel/debug/tegra-host1x/actmon/nvjpg1/module0/usage",
    },
    Engine {
        device: "15340000.vic",
        name: "VIC",
        usage: "/sys/kernel/debug/tegra-host1x/actmon/vic/module0/usage",
    },
    Engine {
        device: "15a50000.ofa",
        name: "OFA",
        usage: "/sys/kernel/debug/tegra-host1x/actmon/ofa/module0/usage",
    },
];
pub(crate) const FIELDS: &[&str] = &[
    "cur_freq",
    "governor",
    "device/power/runtime_status",
    "device/power/runtime_active_time",
    "device/power/runtime_suspended_time",
];

pub(crate) fn source(engine: &Engine, field: &str) -> String {
    if field == "usage" {
        engine.usage.into()
    } else {
        format!("/sys/class/devfreq/{}/{field}", engine.device)
    }
}

pub(crate) fn identify(path: &str) -> Option<(&'static Engine, &'static str)> {
    ENGINES.iter().find_map(|engine| {
        if path == engine.usage {
            return Some((engine, "usage"));
        }
        let rest = path
            .strip_prefix("/sys/class/devfreq/")?
            .strip_prefix(engine.device)?
            .strip_prefix('/')?;
        FIELDS
            .iter()
            .find(|field| **field == rest)
            .map(|field| (engine, *field))
    })
}

pub(crate) fn paths() -> Vec<String> {
    ENGINES
        .iter()
        .flat_map(|engine| {
            FIELDS
                .iter()
                .chain([&"usage"])
                .map(move |field| source(engine, field))
        })
        .collect()
}

pub(crate) fn present(caps: &DeviceCapabilities, engine: &Engine) -> bool {
    caps.tegra
        && FIELDS.iter().any(|field| {
            caps.nodes
                .get(&source(engine, field))
                .is_some_and(|s| *s != MetricStatus::Unsupported)
        })
}

pub(crate) fn probe(caps: &DeviceCapabilities, tracker: &mut TegraTracker) {
    tracker.engines.retain(|device, _| {
        ENGINES
            .iter()
            .any(|e| e.device == *device && present(caps, e))
    });
    tracker.stale();
}

fn parse(field: &str, body: &str) -> Result<String, MetricStatus> {
    let body = body.trim();
    let valid = match field {
        // NVIDIA ACTMON usage 为 0..1000 的归一化读数，不截断越界数据
        "usage" => body.parse::<u16>().is_ok_and(|n| n <= 1000),
        "cur_freq" | "device/power/runtime_active_time" | "device/power/runtime_suspended_time" => {
            body.parse::<u64>().is_ok()
        }
        "device/power/runtime_status" => matches!(
            body,
            "active" | "suspended" | "suspending" | "resuming" | "unsupported" | "error"
        ),
        "governor" => {
            !body.is_empty()
                && body.len() <= 64
                && body
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        }
        _ => false,
    };
    if valid {
        Ok(body.into())
    } else {
        Err(MetricStatus::Failed("unrecognized_format".into()))
    }
}

pub(crate) fn collect(
    output: &str,
    caps: &DeviceCapabilities,
    tracker: &mut TegraTracker,
    now: Instant,
) -> Result<(), MetricStatus> {
    if !caps.tegra {
        tracker.engines.clear();
        return Ok(());
    }
    let parsed = match nodes(output) {
        Ok(parsed) if parsed.iter().all(|node| identify(&node.path).is_some()) => parsed,
        Ok(_) => {
            let status = MetricStatus::Failed("unexpected_tegra_source".into());
            tracker.fail(status.clone());
            return Err(status);
        }
        Err(status) => {
            tracker.fail(status.clone());
            return Err(status);
        }
    };
    let mut timeout = false;
    for engine in ENGINES.iter().filter(|engine| present(caps, engine)) {
        let state = tracker.engines.entry(engine.device).or_default();
        for field in FIELDS.iter().copied().chain(["usage"]) {
            let path = source(engine, field);
            let result = match parsed.iter().find(|node| node.path == path) {
                Some(node) if node.status == MetricStatus::Ready => parse(field, &node.body),
                Some(node) => Err(node.status.clone()),
                None => Err(match caps.nodes.get(&path) {
                    Some(MetricStatus::Ready) => MetricStatus::Unavailable,
                    Some(status) => status.clone(),
                    None => MetricStatus::Unsupported,
                }),
            };
            let metric = state.sources.entry(field.into()).or_default();
            match result {
                Ok(value) => metric.record(value, now),
                Err(status) => {
                    timeout |= status == MetricStatus::Timeout;
                    metric.fail(status);
                }
            }
        }
        state.update_awake(now);
    }
    // 单源格式或权限失败不停止其他公开指标；驱动超时仍遵守重连后重试约定
    if timeout {
        Err(MetricStatus::Timeout)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitor::{
        collectors::{SUDO_NODES, command, parse_probe},
        snapshot::CollectorDomain,
    };
    use std::time::Duration;
    const IDLE: &str = include_str!("../../../tests/fixtures/monitor/jetson-orin-nx-runtime.txt");

    fn capabilities() -> DeviceCapabilities {
        let mut caps = parse_probe(IDLE).unwrap();
        caps.tegra = true;
        for engine in ENGINES {
            caps.nodes
                .insert(engine.usage.into(), MetricStatus::PermissionDenied);
        }
        caps
    }

    fn sample(active: u64, suspended: u64, usage: &str) -> String {
        let engine = &ENGINES[0];
        let mut output = String::new();
        for (field, value) in [
            ("device/power/runtime_status", "active".into()),
            ("device/power/runtime_active_time", active.to_string()),
            ("device/power/runtime_suspended_time", suspended.to_string()),
            ("usage", usage.into()),
        ] {
            output.push_str(&format!(
                "@@node {} ready\n{value}\n@@endnode\n",
                source(engine, field)
            ));
        }
        output.push_str("@@complete");
        output
    }

    #[test]
    fn actual_nx_sleeping_engines_keep_public_data_without_root() {
        let mut tracker = TegraTracker::default();
        collect(IDLE, &capabilities(), &mut tracker, Instant::now()).unwrap();
        assert_eq!(tracker.engines.len(), 6);
        for engine in tracker.engines.values() {
            assert_eq!(
                engine.current("device/power/runtime_status"),
                Some("suspended")
            );
            assert_eq!(
                engine.sources["usage"].status,
                MetricStatus::PermissionDenied
            );
            assert!(engine.awake.value.is_none());
        }
    }

    #[test]
    fn awake_share_resets_on_gaps_rollback_failure_and_stale() {
        let mut tracker = TegraTracker::default();
        let caps = capabilities();
        let now = Instant::now();
        let mut tick = |seconds, active, suspended| {
            collect(
                &sample(active, suspended, "375"),
                &caps,
                &mut tracker,
                now + Duration::from_secs(seconds),
            )
            .unwrap();
            let engine = &tracker.engines[ENGINES[0].device];
            engine.awake.value.filter(|_| !engine.awake.stale)
        };
        assert_eq!(tick(0, 100, 900), None);
        assert_eq!(tick(5, 1100, 4900), Some(20.));
        assert_eq!(tick(10, 0, 0), None);
        assert_eq!(tick(15, 0, 5000), Some(0.));
        assert_eq!(tick(50, 1000, 39000), None);
        tracker.stale();
        collect(
            &sample(2000, 43000, "375"),
            &caps,
            &mut tracker,
            now + Duration::from_secs(55),
        )
        .unwrap();
        assert!(tracker.engines[ENGINES[0].device].awake.stale);
        collect(
            "@@complete",
            &caps,
            &mut tracker,
            now + Duration::from_secs(60),
        )
        .unwrap();
        collect(
            &sample(3000, 47000, "375"),
            &caps,
            &mut tracker,
            now + Duration::from_secs(65),
        )
        .unwrap();
        assert!(tracker.engines[ENGINES[0].device].awake.stale);
    }

    #[test]
    fn usage_rejects_unknown_units_and_preserves_last_success_as_history() {
        let now = Instant::now();
        let caps = capabilities();
        let mut tracker = TegraTracker::default();
        collect(&sample(1, 1, "375"), &caps, &mut tracker, now).unwrap();
        for bad in ["1001", "-1", "NaN", "37.5%", "", "18446744073709551616"] {
            collect(&sample(2, 2, bad), &caps, &mut tracker, now).unwrap();
            let engine = &tracker.engines[ENGINES[0].device];
            assert_eq!(engine.current("usage"), None);
            assert_eq!(engine.sources["usage"].value.as_deref(), Some("375"));
            assert!(engine.sources["usage"].stale);
            assert_eq!(
                engine.current("device/power/runtime_status"),
                Some("active")
            );
        }
        for valid in ["0", "1000"] {
            assert!(parse("usage", valid).is_ok());
        }
    }

    #[test]
    fn partial_timeout_preserves_runtime_but_truncated_transport_invalidates_all() {
        let caps = capabilities();
        let mut tracker = TegraTracker::default();
        let now = Instant::now();
        let frame = format!("@@node {} timeout\n@@endnode\n@@complete", ENGINES[0].usage);
        let output = IDLE.replace("@@complete", &frame);
        assert_eq!(
            collect(&output, &caps, &mut tracker, now),
            Err(MetricStatus::Timeout)
        );
        assert_eq!(
            tracker.engines[ENGINES[0].device].current("device/power/runtime_status"),
            Some("suspended")
        );
        assert!(collect(&output.replace("@@complete", ""), &caps, &mut tracker, now).is_err());
        assert_eq!(
            tracker.engines[ENGINES[0].device].current("device/power/runtime_status"),
            None
        );
    }

    #[test]
    fn authorization_is_fixed_and_does_not_probe_unrelated_boards_or_engines() {
        let mut caps = capabilities();
        assert!(command(&caps, CollectorDomain::Tegra, false).is_some());
        assert!(command(&caps, CollectorDomain::Tegra, true).is_some());
        for engine in ENGINES {
            assert!(SUDO_NODES.contains(&engine.usage));
        }
        assert!(super::super::allowed(ENGINES[0].usage));
        assert!(!super::super::allowed(
            &ENGINES[0].usage.replace("usage", "sample_period")
        ));
        assert!(!super::super::allowed(
            &ENGINES[0].usage.replace("nvenc", "nvenc/..")
        ));
        caps.tegra = false;
        assert!(command(&caps, CollectorDomain::Tegra, true).is_none());
        caps.tegra = true;
        caps.nodes.retain(|path, _| path.contains("/kernel/debug/"));
        assert!(command(&caps, CollectorDomain::Tegra, true).is_none());
    }
    #[test]
    #[ignore = "需要 SHELLRS_MONITOR_TEST_HOST，使用运行中的 ShellRS CLI 只读验证 NX"]
    fn tegra_read_only_smoke() {
        use std::{
            io::Write as _,
            process::{Command, Stdio},
        };
        let host = std::env::var("SHELLRS_MONITOR_TEST_HOST").expect("explicit NX host ID");
        let run = |command: String| {
            let mut child = Command::new("shellrs")
                .args(["exec", "--json"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let request = serde_json::json!({"host": host, "command": command});
            child
                .stdin
                .take()
                .unwrap()
                .write_all(request.to_string().as_bytes())
                .unwrap();
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(result["exit_code"], 0);
            result["stdout"].as_str().unwrap().to_owned()
        };
        let caps = parse_probe(&run(super::super::probe_command())).unwrap();
        assert!(caps.tegra);
        let command = command(&caps, CollectorDomain::Tegra, false).expect("public NX nodes");
        let mut tracker = TegraTracker::default();
        collect(&run(command), &caps, &mut tracker, Instant::now()).unwrap();
        assert!(!tracker.engines.is_empty());
        for (device, state) in tracker.engines {
            assert!(state.current("device/power/runtime_status").is_some());
            assert!(state.current("cur_freq").is_some());
            println!(
                "{device}: runtime={}, ACTMON={:?}",
                state.current("device/power/runtime_status").unwrap(),
                state.sources["usage"].status
            );
        }
    }
}
