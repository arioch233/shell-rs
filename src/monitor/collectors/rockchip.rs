//! Rockchip 输出中的硬件身份和数值独立验证，额外业务进程文本不保留

use super::{DeviceCapabilities, domain, nodes};
use crate::monitor::snapshot::{CollectorDomain, HardwareMetric, MetricStatus, Unit};
use std::collections::HashSet;

fn percent(text: &str) -> Option<f64> {
    let value: f64 = text.trim().trim_end_matches('%').trim().parse().ok()?;
    (value.is_finite() && (0.0..=100.0).contains(&value)).then_some(value)
}

fn malformed() -> MetricStatus {
    MetricStatus::Failed("unrecognized_format".into())
}

fn rga(body: &str, source: &str) -> Result<Vec<HardwareMetric>, MetricStatus> {
    let mut metrics = Vec::new();
    let mut ids = HashSet::new();
    let mut current = None;
    let mut count = None;
    for line in body.lines().map(str::trim) {
        if let Some(value) = line.strip_prefix("num of scheduler") {
            count = Some(
                value
                    .trim()
                    .strip_prefix('=')
                    .and_then(|s| s.trim().parse::<usize>().ok())
                    .ok_or_else(malformed)?,
            );
        }
        if let Some(rest) = line.strip_prefix("scheduler[") {
            if current.is_some() {
                return Err(malformed());
            }
            let (id, kind) = rest.split_once("]:").ok_or_else(malformed)?;
            let id: u32 = id.parse().map_err(|_| malformed())?;
            if !ids.insert(id) {
                return Err(malformed());
            }
            current = Some((id, kind.trim().to_owned()));
        } else if let Some(value) = line.strip_prefix("load") {
            let Some((id, kind)) = current.take() else {
                continue;
            };
            let value = value
                .trim()
                .strip_prefix('=')
                .and_then(percent)
                .ok_or_else(malformed)?;
            metrics.push(HardwareMetric::new(
                format!("rockchip/rga/scheduler/{id}"),
                format!("{kind} · {id}"),
                value,
                Unit::Percent,
                source,
            ));
        }
    }
    if metrics.is_empty() || current.is_some() || count.is_some_and(|count| count != metrics.len())
    {
        return Err(malformed());
    }
    Ok(metrics)
}

fn npu(body: &str, source: &str) -> Result<Vec<HardwareMetric>, MetricStatus> {
    let mut metrics = Vec::new();
    let mut ids = HashSet::new();
    for core in body.split("Core").skip(1) {
        let (id, rest) = core.split_once(':').ok_or_else(malformed)?;
        let id: u32 = id.trim().parse().map_err(|_| malformed())?;
        let (value, _) = rest.split_once('%').ok_or_else(malformed)?;
        if !ids.insert(id) {
            return Err(malformed());
        }
        metrics.push(HardwareMetric::new(
            format!("rockchip/npu/core/{id}"),
            format!("NPU · Core{id}"),
            percent(value).ok_or_else(malformed)?,
            Unit::Percent,
            source,
        ));
    }
    if metrics.is_empty() {
        if let Some(value) = body.trim().strip_prefix("NPU load:").and_then(percent) {
            metrics.push(HardwareMetric::new(
                "rockchip/npu/core/0",
                "NPU · Core0",
                value,
                Unit::Percent,
                source,
            ));
        } else {
            return Err(malformed());
        }
    }
    Ok(metrics)
}

fn mpp(body: &str, source: &str) -> Result<Vec<HardwareMetric>, MetricStatus> {
    if body.contains("set load_interval") {
        return Ok(vec![HardwareMetric::state(
            source,
            MetricStatus::Unavailable,
            "load_interval=0",
        )]);
    }
    let mut result = Vec::new();
    let mut ids = HashSet::new();
    for line in body.lines().filter(|line| !line.trim().is_empty()) {
        let words: Vec<_> = line.split_whitespace().collect();
        let Some(name) = words.first() else { continue };
        if !words.contains(&"load:") || !words.contains(&"utilization:") {
            return Err(malformed());
        }
        if !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        {
            return Err(malformed());
        }
        for (ix, key) in words.iter().enumerate() {
            if !matches!(*key, "load:" | "utilization:") {
                continue;
            }
            let id = format!("rockchip/mpp/{name}/{}", key.trim_end_matches(':'));
            if !ids.insert(id.clone()) {
                return Err(malformed());
            }
            let value = words
                .get(ix + 1)
                .and_then(|value| percent(value))
                .ok_or_else(malformed)?;
            result.push(HardwareMetric::new(
                id,
                format!("{name} · {}", key.trim_end_matches(':')),
                value,
                Unit::Percent,
                source,
            ));
        }
    }
    if result.is_empty() {
        return Err(malformed());
    }
    Ok(result)
}

fn parse_node(
    path: &str,
    body: &str,
    all: &[super::NodeOutput],
) -> Result<Vec<HardwareMetric>, MetricStatus> {
    if path == "/sys/kernel/debug/rkrga/load" {
        return rga(body, path);
    }
    if path == "/sys/kernel/debug/rknpu/load" {
        return npu(body, path);
    }
    if path == "/proc/mpp_service/load" {
        return mpp(body, path);
    }
    if path.contains("/devfreq/") {
        let name = path.split('/').nth(4).ok_or_else(malformed)?;
        if path.ends_with("/name") || path.ends_with("/gpuinfo") {
            return Ok(vec![HardwareMetric::state(
                path,
                MetricStatus::Ready,
                body.trim(),
            )]);
        }
        let (value, unit) = if path.ends_with("/cur_freq") {
            let value = body.trim().parse::<u64>().map_err(|_| malformed())?;
            (value as f64, Unit::Hertz)
        } else {
            let text = body.trim().split('@').next().unwrap_or("");
            (percent(text).ok_or_else(malformed)?, Unit::Percent)
        };
        return Ok(vec![HardwareMetric::new(
            path,
            format!("{name} · devfreq"),
            value,
            unit,
            path,
        )]);
    }
    if path.contains("/thermal/") {
        if path.ends_with("/type") {
            return Ok(Vec::new());
        }
        let name_path = path.replace("/temp", "/type");
        let label = all
            .iter()
            .find(|node| node.path == name_path && node.status == MetricStatus::Ready)
            .map_or(path, |node| node.body.trim());
        let value: f64 = body.trim().parse().map_err(|_| malformed())?;
        if !value.is_finite() || !(-100000.0..=250000.0).contains(&value) {
            return Err(malformed());
        }
        return Ok(vec![HardwareMetric::new(
            path,
            label,
            value / 1000.0,
            Unit::Celsius,
            path,
        )]);
    }
    if path.contains("/pressure/") {
        let mut result = Vec::new();
        // 采集协议会在节点正文与退出状态之间补换行
        for line in body.lines().filter(|line| !line.trim().is_empty()) {
            let mut words = line.split_whitespace();
            let kind = words.next().ok_or_else(malformed)?;
            if !matches!(kind, "some" | "full") || (path.ends_with("/cpu") && kind == "full") {
                continue;
            }
            let value = words
                .find_map(|word| word.strip_prefix("avg10="))
                .and_then(percent)
                .ok_or_else(malformed)?;
            result.push(HardwareMetric::new(
                format!("{path}/{kind}"),
                format!(
                    "PSI {} · {kind} avg10",
                    path.rsplit('/').next().unwrap_or("")
                ),
                value,
                Unit::Percent,
                path,
            ));
        }
        if result.is_empty() {
            return Err(malformed());
        }
        return Ok(result);
    }
    if path == super::mpp::CLOCK_SOURCE || path.ends_with("/sessions-info") {
        // 结构化 MPP 采集器保留所需字段，通用硬件列表不复制整份诊断文本
        return Ok(vec![HardwareMetric::state(path, MetricStatus::Ready, "")]);
    }
    if path.contains("sessions") {
        let summary = body
            .lines()
            .filter(|line| {
                line.trim().starts_with("session:")
                    || line.trim().starts_with("device:")
                    || line.trim().starts_with("memory:")
            })
            .map(str::trim)
            .collect::<Vec<_>>()
            .join("\n");
        return Ok(vec![HardwareMetric::state(
            path,
            MetricStatus::Ready,
            summary,
        )]);
    }
    if path.ends_with("/hardware") {
        return Ok(Vec::new());
    }
    Ok(vec![HardwareMetric::state(
        path,
        MetricStatus::Ready,
        body.trim(),
    )])
}

pub(crate) fn parse_nodes(
    output: &str,
    caps: &DeviceCapabilities,
    requested: CollectorDomain,
) -> Result<Vec<HardwareMetric>, MetricStatus> {
    let parsed = nodes(output)?;
    let mut result = Vec::new();
    for node in &parsed {
        if domain(&node.path) != Some(requested) {
            return Err(malformed());
        }
        // 驱动版本节点有两种命名，缺失的别名不作为硬件故障展示
        if node.status == MetricStatus::Unsupported
            && matches!(
                node.path.as_str(),
                "/sys/kernel/debug/rknpu/version" | "/sys/kernel/debug/rknpu/driver_version"
            )
        {
            continue;
        }
        if node.status != MetricStatus::Ready {
            result.push(HardwareMetric::state(&node.path, node.status.clone(), ""));
        } else {
            match parse_node(&node.path, &node.body, &parsed) {
                Ok(metrics) => result.extend(metrics),
                Err(status) => result.push(HardwareMetric::state(&node.path, status, "")),
            }
        }
    }
    for (path, status) in &caps.nodes {
        if domain(path) == Some(requested)
            && *status != MetricStatus::Ready
            && !parsed.iter().any(|node| node.path == *path)
        {
            // 不存在的版本等辅助节点不制造占位，核心负载和权限问题保留
            if *status == MetricStatus::PermissionDenied || path.ends_with("/load") {
                result.push(HardwareMetric::state(path, status.clone(), ""));
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gpu_identity_nodes_preserve_text_without_interpreting_core_count_as_load() {
        let rows = parse_nodes(
            "@@node /sys/class/devfreq/fb000000.gpu/name ready\nfb000000.gpu\n@@endnode\n@@node /sys/class/devfreq/fb000000.gpu/device/gpuinfo ready\nMali-G610 4 cores r0p0 0xA8670000\n@@endnode\n@@complete",
            &DeviceCapabilities::default(), CollectorDomain::Devfreq,
        ).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].detail, "fb000000.gpu");
        assert_eq!(rows[1].detail, "Mali-G610 4 cores r0p0 0xA8670000");
        assert!(rows.iter().all(|row| row.unit == Unit::Text
            && row.value.is_none()
            && row.status == MetricStatus::Ready));
    }

    #[test]
    fn npu_driver_version_accepts_both_node_names_without_missing_alias_errors() {
        for (present, absent) in [("version", "driver_version"), ("driver_version", "version")] {
            let path = format!("/sys/kernel/debug/rknpu/{present}");
            let output = format!(
                "@@node {path} ready\nRKNPU driver: v0.9.8\n@@endnode\n@@node /sys/kernel/debug/rknpu/{absent} unsupported\n@@endnode\n@@complete"
            );
            let rows = parse_nodes(
                &output,
                &DeviceCapabilities::default(),
                CollectorDomain::Npu,
            )
            .unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].source, path);
            assert_eq!(rows[0].detail, "RKNPU driver: v0.9.8");
            assert_eq!(rows[0].status, MetricStatus::Ready);
            assert_eq!(rows[0].value, None);
        }
    }
    #[test]
    fn sanitized_board_samples_and_source_derived_mpp_format() {
        let rga = rga(
            include_str!("../../../tests/fixtures/monitor/rk3588-5.10-rga.txt"),
            "rga",
        )
        .unwrap();
        assert_eq!(
            rga.iter().map(|row| row.value).collect::<Vec<_>>(),
            vec![Some(2.0), Some(0.0), Some(0.0)]
        );
        assert_eq!(
            npu(
                include_str!("../../../tests/fixtures/monitor/rk3588-5.10-npu.txt"),
                "npu"
            )
            .unwrap()
            .len(),
            3
        );
        let rows = parse_nodes(
            include_str!("../../../tests/fixtures/monitor/rk3588-5.10-mpp.txt"),
            &DeviceCapabilities::default(),
            CollectorDomain::Mpp,
        )
        .unwrap();
        assert_eq!(rows[1].status, MetricStatus::Unsupported);
        assert_eq!(rows[1].value, None);
        let rows = mpp(
            include_str!("../../../tests/fixtures/monitor/mpp-6.1-source.txt"),
            "mpp",
        )
        .unwrap();
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].value, Some(10.25));
        assert_eq!(rows[1].value, Some(42.0));
    }

    #[test]
    fn dynamic_rga_and_npu_keep_ids_and_reject_invalid_values() {
        let data = "num of scheduler = 2\nscheduler[7]: rga3\n load = 0%\nscheduler[9]: rga2\n load = 34%\n";
        let rows = rga(data, "rga").unwrap();
        assert!(rows[0].id.ends_with("/7"));
        assert_eq!(rows[0].value, Some(0.0));
        assert!(rga(&data.replace("34%", "101%"), "rga").is_err());
        assert!(rga(&data.replace("= 2", "= 3"), "rga").is_err());
        assert_eq!(
            npu("NPU load: Core0: 1%, Core1: 0%, Core2: 0%,", "npu")
                .unwrap()
                .len(),
            3
        );
        assert_eq!(npu("NPU load: 0%", "npu").unwrap().len(), 1);
        assert!(npu("NPU load: Core0: 0%, Core0: 1%", "npu").is_err());
    }
    #[test]
    fn mpp_never_invents_load_and_keeps_both_measures() {
        let rows = mpp(
            "fdc38100.rkvdec-core load: 10.25% utilization: 42.00%",
            "mpp",
        )
        .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].value, Some(42.0));
        assert_eq!(
            mpp("please set load_interval first!!!", "mpp").unwrap()[0].status,
            MetricStatus::Unavailable
        );
        assert!(mpp("session: pid=10 index=1", "mpp").is_err());
    }
    #[test]
    fn psi_accepts_real_collector_newlines_but_rejects_invalid_measurements() {
        // 实机输出包含读取命令在 @@status 前补的分隔换行
        let rows = parse_nodes(
            include_str!("../../../tests/fixtures/monitor/linux-5.15-psi.txt"),
            &DeviceCapabilities::default(),
            CollectorDomain::Psi,
        )
        .unwrap();
        assert_eq!(rows.len(), 5);
        assert!(rows.iter().all(|row| row.status == MetricStatus::Ready));
        assert_eq!(
            rows.iter().map(|row| row.value).collect::<Vec<_>>(),
            [Some(0.12), Some(0.), Some(0.), Some(2.93), Some(2.93)]
        );
        assert!(!rows.iter().any(|row| row.id == "/proc/pressure/cpu/full"));
        for body in [
            "",
            "\n  \n",
            "some avg10=NaN\n\n",
            "some avg10=101.00\n\n",
            "some avg60=2.00\n\n",
        ] {
            assert!(parse_node("/proc/pressure/cpu", body, &[]).is_err());
        }
        let rows = parse_node(
            "/proc/pressure/cpu",
            " \n some avg10=0.00 avg60=0.00\n\t\n",
            &[],
        )
        .unwrap();
        assert_eq!(rows[0].value, Some(0.));
    }

    #[test]
    fn devfreq_and_partial_psi_are_explicit() {
        let rows = parse_node("/sys/class/devfreq/gpu/load", "35@400000000Hz", &[]).unwrap();
        assert_eq!(rows[0].value, Some(35.0));
        let rows = parse_node(
            "/proc/pressure/cpu",
            "some avg10=1.20 avg60=2.00\nfull avg10=0.00",
            &[],
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
    }
}
