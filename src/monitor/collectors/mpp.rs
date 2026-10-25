//! MPP 表格、任务索引与时钟的纯解析，不从时钟或任务数推算负载

use std::collections::{BTreeMap, HashSet};

use super::{DeviceCapabilities, NodeOutput, domain, nodes};
use crate::monitor::{
    mpp::{CodecKind, MppClock, MppReading, MppSession, MppTracker},
    snapshot::{CollectorDomain, MetricStatus},
};
use std::time::Instant;

pub(crate) const CLOCK_SOURCE: &str = "/sys/kernel/debug/clk/clk_summary";

fn malformed() -> MetricStatus {
    MetricStatus::Failed("unrecognized_format".into())
}

fn kind(name: &str) -> CodecKind {
    let name = name.to_ascii_lowercase();
    if name.contains("enc") || name.contains("vepu") {
        CodecKind::Encoder
    } else if name.contains("dec") || name.contains("vdpu") {
        CodecKind::Decoder
    } else {
        CodecKind::Other
    }
}

fn number(text: &str) -> Result<Option<f64>, MetricStatus> {
    if matches!(text.trim(), "" | "-" | "--" | "N/A" | "n/a") {
        return Ok(None);
    }
    let value: f64 = text.parse().map_err(|_| malformed())?;
    if value.is_finite() && value >= 0. {
        Ok(Some(value))
    } else {
        Err(malformed())
    }
}

fn sessions(body: &str) -> Result<Vec<MppSession>, MetricStatus> {
    let mut result = Vec::<MppSession>::new();
    let mut columns = Vec::<&str>::new();
    let mut table_rows = BTreeMap::<u64, BTreeMap<String, String>>::new();
    let mut ids = HashSet::new();
    for line in body.lines().map(str::trim) {
        if let Some(header) = line.strip_prefix("session:") {
            let fields: BTreeMap<_, _> = header
                .split_whitespace()
                .filter_map(|s| s.split_once('='))
                .collect();
            let pid = fields
                .get("pid")
                .ok_or_else(malformed)?
                .parse()
                .map_err(|_| malformed())?;
            let index = fields
                .get("index")
                .ok_or_else(malformed)?
                .parse()
                .map_err(|_| malformed())?;
            if !ids.insert(index) {
                return Err(malformed());
            }
            result.push(MppSession {
                pid,
                index,
                device: String::new(),
                kind: CodecKind::Other,
                memory_bytes: None,
                fields: BTreeMap::new(),
                fps: None,
            });
        } else if let Some(device) = line.strip_prefix("device:") {
            let session = result.last_mut().ok_or_else(malformed)?;
            session.device = device.trim().to_owned();
            session.kind = kind(&session.device);
        } else if let Some(memory) = line.strip_prefix("memory:") {
            let session = result.last_mut().ok_or_else(malformed)?;
            let mut words = memory.split_whitespace();
            let value: u64 = words
                .next()
                .ok_or_else(malformed)?
                .parse()
                .map_err(|_| malformed())?;
            let multiplier = match words.next() {
                Some("MiB") => 1024 * 1024,
                Some("KiB") => 1024,
                Some("Bytes" | "B") => 1,
                _ => return Err(malformed()),
            };
            session.memory_bytes = Some(value.checked_mul(multiplier).ok_or_else(malformed)?);
        } else if line.starts_with('|') {
            let cells: Vec<_> = line.trim_matches('|').split('|').map(str::trim).collect();
            if cells.contains(&"session") {
                if cells.iter().collect::<HashSet<_>>().len() != cells.len() {
                    return Err(malformed());
                }
                columns = cells;
                continue;
            }
            if cells
                .iter()
                .all(|cell| cell.bytes().all(|b| b"-+: ".contains(&b)))
            {
                continue;
            }
            if columns.is_empty() || columns.len() != cells.len() {
                return Err(malformed());
            }
            let fields: BTreeMap<_, _> = columns.iter().copied().zip(cells).collect();
            let index: u64 = fields
                .get("session")
                .ok_or_else(malformed)?
                .parse()
                .map_err(|_| malformed())?;
            let mut values = BTreeMap::new();
            for (key, value) in fields {
                if matches!(
                    key,
                    "width" | "height" | "fps_in" | "fps_out" | "fps_calc" | "bitrate" | "gop_size"
                ) {
                    if number(value)?.is_some() {
                        if matches!(key, "width" | "height" | "bitrate" | "gop_size") {
                            value.parse::<u64>().map_err(|_| malformed())?;
                        }
                        values.insert(key.to_owned(), value.to_owned());
                    }
                } else if matches!(key, "device" | "format" | "rc_mode" | "profile") {
                    values.insert(key.to_owned(), value.to_owned());
                }
            }
            if table_rows.insert(index, values).is_some() {
                return Err(malformed());
            }
        }
    }
    // 无会话时驱动输出为空；非空未知文本不能伪装成零会话
    if result.is_empty() && !body.trim().is_empty() {
        return Err(malformed());
    }
    for session in &mut result {
        if session.device.is_empty() {
            return Err(malformed());
        }
        if let Some(fields) = table_rows.remove(&session.index) {
            if let Some(device) = fields.get("device") {
                session.kind = kind(device);
            }
            session.fps = fields
                .get("fps_calc")
                .map(|value| number(value))
                .transpose()?
                .flatten();
            session.fields = fields;
        }
    }
    if !table_rows.is_empty() {
        return Err(malformed());
    }
    Ok(result)
}

fn clock_core(name: &str) -> Option<String> {
    for engine in ["rkvenc", "rkvdec"] {
        for (prefix, suffix) in [("clk_", "_core"), ("aclk_", ""), ("hclk_", "")] {
            if let Some(index) = name
                .strip_prefix(&format!("{prefix}{engine}"))
                .and_then(|s| s.strip_suffix(suffix))
                .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            {
                return Some(format!("{engine}-core{index}"));
            }
        }
    }
    None
}

fn clocks(body: &str) -> Result<Vec<MppClock>, MetricStatus> {
    let mut rate_column = None;
    let mut result = Vec::new();
    let mut names = HashSet::new();
    for line in body.lines() {
        let words: Vec<_> = line.split_whitespace().collect();
        if words.first() == Some(&"clock") {
            rate_column = words.iter().position(|word| *word == "rate");
            continue;
        }
        let Some(name) = words.first() else { continue };
        let Some(core) = clock_core(name) else {
            continue;
        };
        if !names.insert(*name) {
            return Err(malformed());
        }
        let rate: u64 = words
            .get(rate_column.ok_or_else(malformed)?)
            .ok_or_else(malformed)?
            .parse()
            .map_err(|_| malformed())?;
        let enable_count = words
            .get(1)
            .ok_or_else(malformed)?
            .parse()
            .map_err(|_| malformed())?;
        result.push(MppClock {
            name: (*name).into(),
            core,
            rate_hz: rate,
            enable_count,
        });
    }
    if rate_column.is_none() {
        return Err(malformed());
    }
    Ok(result)
}

fn parse(node: &NodeOutput) -> Result<MppReading, MetricStatus> {
    if node.status != MetricStatus::Ready {
        return Err(node.status.clone());
    }
    if node.body.len() > 262144 {
        return Err(MetricStatus::Failed("output_limit".into()));
    }
    let path = node.path.as_str();
    if path.ends_with("/sessions-summary") || path.ends_with("/sessions_summary") {
        return sessions(&node.body).map(MppReading::Sessions);
    }
    if path == CLOCK_SOURCE {
        return clocks(&node.body).map(MppReading::Clocks);
    }
    if path.ends_with("/task_count") {
        return node
            .body
            .trim()
            .parse()
            .map(|index| MppReading::Tasks {
                index,
                per_second: None,
            })
            .map_err(|_| malformed());
    }
    if path.ends_with("/sessions-info") {
        let mut end = node.body.len().min(2048);
        while !node.body.is_char_boundary(end) {
            end -= 1;
        }
        return Ok(MppReading::Diagnostic {
            bytes: node.body.len(),
            preview: node.body[..end].into(),
        });
    }
    Ok(MppReading::Text(node.body.trim().into()))
}

pub(in crate::monitor) fn collect(
    output: &str,
    caps: &DeviceCapabilities,
    tracker: &mut MppTracker,
    now: Instant,
) -> Result<(), MetricStatus> {
    let mut parsed = nodes(output)?;
    if parsed
        .iter()
        .any(|node| domain(&node.path) != Some(CollectorDomain::Mpp))
    {
        return Err(malformed());
    }
    for (path, status) in &caps.nodes {
        if domain(path) == Some(CollectorDomain::Mpp)
            && !parsed.iter().any(|node| &node.path == path)
        {
            parsed.push(NodeOutput {
                path: path.clone(),
                body: String::new(),
                status: if *status == MetricStatus::Ready {
                    MetricStatus::Unavailable
                } else {
                    status.clone()
                },
            });
        }
    }
    tracker
        .sources
        .retain(|path, _| parsed.iter().any(|node| &node.path == path));
    let mut timeout = false;
    let mut sudo_error = None;
    for node in parsed {
        if node.path == "/proc/mpp_service/load" || (node.path == CLOCK_SOURCE && !caps.has_mpp()) {
            continue;
        }
        timeout |= node.status == MetricStatus::Timeout;
        if matches!(
            node.status,
            MetricStatus::SudoDenied | MetricStatus::SudoUnavailable
        ) {
            sudo_error = Some(node.status.clone());
        }
        tracker.record(&node.path, parse(&node), now);
    }
    if timeout {
        return Err(MetricStatus::Timeout);
    }
    if let Some(error) = sudo_error {
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_table_joins_by_id_and_distinguishes_configured_and_actual_fps() {
        let data = sessions(include_str!(
            "../../../tests/fixtures/monitor/mpp-sessions-summary.txt"
        ))
        .unwrap();
        assert_eq!(data.len(), 2);
        let encoder = &data[0];
        assert_eq!(
            (encoder.pid, encoder.index, encoder.kind),
            (1354197, 495, CodecKind::Encoder)
        );
        assert_eq!(encoder.fields["fps_in"], "21");
        assert_eq!(encoder.fields["fps_out"], "21");
        assert_eq!(encoder.fps, Some(12.95));
        assert_eq!(encoder.fields["bitrate"], "22707504");
        assert_eq!(data[1].kind, CodecKind::Decoder);
        assert_eq!(data[1].fps, None);
        assert_eq!(data[1].memory_bytes, Some(16 * 1024 * 1024));
        let reordered = "session: pid=10 index=8\n device: enc\nsession: pid=11 index=9\n device: enc\n| fps_calc | format | session | device | fps_out |\n| 0 | h265 | 9 | RKVENC | 30 |\n| N/A | h264 | 8 | RKVENC | 21 |";
        let data = sessions(reordered).unwrap();
        assert_eq!(data[0].fps, None);
        assert_eq!(data[1].fps, Some(0.));
        assert_eq!(data[1].fields["format"], "h265");
    }

    #[test]
    fn missing_malformed_or_ambiguous_sessions_are_not_zero_sessions() {
        assert!(sessions("").unwrap().is_empty());
        for body in [
            "unrecognized output",
            "session: pid=1 index=1",
            "| session | fps_calc |\n| 9 | 5 |",
            "session: pid=1 index=1\ndevice: enc\nsession: pid=2 index=1\ndevice: enc",
            "session: pid=1 index=1\ndevice: enc\n| session | fps_calc |\n| 1 | NaN |",
            "session: pid=1 index=1\ndevice: enc\n| session | fps_calc |\n| 1 | -1 |",
            "session: pid=1 index=1\ndevice: enc\n| session | fps_calc |\n| 1 |",
        ] {
            assert!(sessions(body).is_err(), "{body}");
        }
    }

    #[test]
    fn clock_columns_follow_header_and_never_turn_enable_count_into_usage() {
        let data = clocks(include_str!(
            "../../../tests/fixtures/monitor/mpp-clk-summary.txt"
        ))
        .unwrap();
        assert_eq!(data.len(), 6);
        assert_eq!(data[0].core, "rkvenc-core0");
        assert_eq!(data[0].rate_hz, 800_000_000);
        assert_eq!(data[3].enable_count, 0);
        assert_eq!(data[3].rate_hz, 600_000_000);
        let data =
            clocks("clock count count rate accuracy phase\nclk_rkvdec7_core 2 1 400000000 0 0")
                .unwrap();
        assert_eq!(data[0].rate_hz, 400_000_000);
        assert_eq!(data[0].core, "rkvdec-core7");
        assert!(clocks("clk_rkvdec0_core 1 1 0 500000000").is_err());
        assert!(clocks("clock count count rate\nclk_rkvdec0_core 1 1").is_err());
    }

    #[test]
    fn diagnostics_are_bounded_and_missing_reads_do_not_keep_rates_alive() {
        let body = "诊断".repeat(600);
        let reading = parse(&NodeOutput {
            path: "/proc/mpp_service/rkvenc-core0/sessions-info".into(),
            status: MetricStatus::Ready,
            body: body.clone(),
        })
        .unwrap();
        let MppReading::Diagnostic { bytes, preview } = reading else {
            panic!("expected diagnostic")
        };
        assert_eq!(bytes, body.len());
        assert!(preview.len() <= 2048 && body.starts_with(&preview));
        let mut caps = DeviceCapabilities::default();
        let path = "/proc/mpp_service/rkvdec-core0/task_count";
        caps.nodes.insert(path.into(), MetricStatus::Ready);
        let mut tracker = MppTracker::default();
        let now = Instant::now();
        collect(
            &format!("@@node {path} ready\n100\n@@endnode\n@@complete"),
            &caps,
            &mut tracker,
            now,
        )
        .unwrap();
        collect("@@complete", &caps, &mut tracker, now).unwrap();
        assert_eq!(tracker.sources[path].status, MetricStatus::Unavailable);
        assert!(tracker.sources[path].stale);
        collect(
            &format!("@@node {path} ready\n110\n@@endnode\n@@complete"),
            &caps,
            &mut tracker,
            now + std::time::Duration::from_secs(2),
        )
        .unwrap();
        assert_eq!(
            tracker.sources[path].value,
            Some(MppReading::Tasks {
                index: 110,
                per_second: None
            })
        );
    }
}
