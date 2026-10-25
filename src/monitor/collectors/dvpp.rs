//! 310B 的 HiDVPP 分节表格解析，帧率和计数不换算为硬件利用率

use std::{
    collections::{BTreeMap, HashSet},
    time::Instant,
};

use super::{DeviceCapabilities, nodes};
use crate::monitor::{
    dvpp::{DvppChannel, DvppTracker},
    snapshot::MetricStatus,
};

pub(crate) const SOURCES: &[&str] = &[
    "/proc/umap/vdec",
    "/proc/umap/venc",
    "/proc/umap/vpc",
    "/proc/umap/h264e",
    "/proc/umap/h265e",
    "/proc/umap/rc",
];

// 表头随驱动版本扩展，按分节名称和列名读取，不依赖固定列位置
fn fields(source: &str, section: &str) -> &'static [(&'static str, &'static str)] {
    match (source, section) {
        ("vdec", "CHN COMM ATTR & PARAMS") => &[
            ("TYPE", "codec"),
            ("Width", "width"),
            ("Height", "height"),
            ("STATE", "state"),
            ("StrBufSize", "stream_buffer"),
            ("FrmBufCnt", "frame_buffers"),
        ],
        ("vdec", "CHN STATE") => &[
            ("fps", "decode_fps"),
            ("Sendfps", "send_fps"),
            ("DataLen", "queued_bytes"),
            ("PrtclErr", "protocol_errors"),
            ("StrmError", "stream_errors"),
            ("PicSizeErr", "size_errors"),
        ],
        ("vdec", "Detail Stream STATE") => &[
            ("MpiSndNum", "submitted"),
            ("VdecNum", "decoded"),
            ("FmGetNum", "retrieved"),
            ("FmRlsNum", "released"),
        ],
        ("vdec", "Detail FrameStore STATE") => &[
            ("Discard", "discarded"),
            ("FrmInVdec", "queued_frames"),
            ("BusyNode", "busy_buffers"),
        ],
        ("vdec", "Detail Vdec Cost Time") => &[
            ("AveTask(us)", "average_task_us"),
            ("MaxTask(us)", "maximum_task_us"),
            ("MinTask(us)", "minimum_task_us"),
        ],
        ("vdec", "Detail Vdec Chn Id Info")
        | ("venc", "Detail Venc Chn Id Info")
        | ("vpc", "Detail Vpc Chn Id Info") => &[
            ("Pid", "pid"),
            ("UserChnId", "user_channel"),
            ("VFID", "vfid"),
        ],
        ("venc", "VENC CHN ATTR 1") => &[
            ("Type", "codec"),
            ("Width", "width"),
            ("Height", "height"),
            ("LeftBytes", "queued_bytes"),
            ("LeftFrm", "queued_frames"),
        ],
        ("venc", "VENC CHN ATTR 2") => &[("SrcFr", "source_fps"), ("TarFr", "target_fps")],
        ("venc", "VENC STREAM STATE") => &[
            ("FrameRate", "driver_fps"),
            ("BusyCnt", "busy_buffers"),
            ("UserGet", "retrieved"),
            ("UserRls", "released"),
        ],
        ("venc", "VENC CHNL INFO") => &[
            ("OverLoad", "overloads"),
            ("RingSkip", "ring_skips"),
            ("RcSkip", "rc_skips"),
        ],
        ("venc", "VENC TASK COST TIME") => &[
            ("AveTask(us)", "average_task_us"),
            ("MaxTask(us)", "maximum_task_us"),
            ("MinTask(us)", "minimum_task_us"),
        ],
        ("h264e" | "h265e", "CHN ATTR") => &[("Width", "width"), ("Height", "height")],
        ("h264e" | "h265e", "PICTURE INFO") => &[
            ("EncStart", "encode_started"),
            ("EncSucceed", "encode_succeeded"),
            ("Lost", "lost"),
            ("Disc", "discarded"),
            ("Recode", "reencoded"),
        ],
        ("rc", "BASE PARAMS 1") => &[
            ("Gop", "gop"),
            ("ViFr", "source_fps"),
            ("TrgFr", "target_fps"),
            ("RcMode", "rc_mode"),
            ("Br(kbps)", "configured_kbps"),
        ],
        ("rc", "RUN INFO1") => &[("InsBr(kbps)", "instant_kbps"), ("InsFr", "rc_fps")],
        ("vpc", "CHN PIC ATTR & PARAMS") => &[
            ("input_w", "width"),
            ("input_h", "height"),
            ("output_w", "output_width"),
            ("output_h", "output_height"),
        ],
        ("vpc", "CHN FUNC ATTR & PARAMS") => &[
            ("Func_type", "function"),
            ("count", "operations"),
            ("general_mem/byte", "memory_bytes"),
        ],
        ("vpc", "CHN QUEUE STATUS") => &[
            ("working_queue", "working_queue"),
            ("event_queue", "event_queue"),
        ],
        ("vpc", "Detail Vpc Cost Time") => &[
            ("SendFrameRate(fps)", "send_fps"),
            ("AveTask(us)", "average_task_us"),
            ("MaxTask(us)", "maximum_task_us"),
            ("MinTask(us)", "minimum_task_us"),
        ],
        ("vpc", "Detail Vpc Task Cnt Info") => &[
            ("add_task_cnt", "submitted"),
            ("get_result_cnt", "results"),
            ("recv_err_cnt", "interrupt_errors"),
            ("retry_cnt", "retries"),
        ],
        _ => &[],
    }
}

fn malformed() -> MetricStatus {
    MetricStatus::Failed("unrecognized_dvpp_format".into())
}

fn value(key: &str, text: &str) -> Result<Option<String>, MetricStatus> {
    if matches!(text, "-" | "--" | "N/A" | "NA") {
        return Ok(None);
    }
    if matches!(key, "codec" | "state" | "rc_mode" | "function") {
        return (text.len() <= 64 && text.bytes().all(|b| b.is_ascii_graphic()))
            .then(|| Some(text.to_owned()))
            .ok_or_else(malformed);
    }
    let valid_number = |s: &str| s.parse::<f64>().is_ok_and(|n| n.is_finite() && n >= 0.);
    let valid = if key.ends_with("fps") {
        if let Some((n, d)) = text.split_once('/') {
            valid_number(n) && d.parse::<f64>().is_ok_and(|d| d.is_finite() && d > 0.)
        } else {
            valid_number(text)
        }
    } else if key.ends_with("_us") || key.ends_with("_kbps") {
        valid_number(text)
    } else {
        text.parse::<u64>().is_ok()
    };
    valid.then(|| Some(text.to_owned())).ok_or_else(malformed)
}

pub(super) fn parse(source: &str, body: &str) -> Result<Vec<DvppChannel>, MetricStatus> {
    if body.len() > 262144 {
        return Err(MetricStatus::Failed("output_limit".into()));
    }
    let name = source.rsplit('/').next().ok_or_else(malformed)?;
    let primary = match name {
        "vdec" => "CHN COMM ATTR & PARAMS",
        "venc" => "VENC CHN ATTR 1",
        "vpc" => "CHN PIC ATTR & PARAMS",
        "h264e" | "h265e" => "CHN ATTR",
        "rc" => "BASE PARAMS 1",
        _ => return Err(malformed()),
    };
    let mut section = "";
    let mut columns = Vec::new();
    let mut selected = Vec::new();
    let mut table_ids = HashSet::new();
    let mut channels = BTreeMap::<u32, DvppChannel>::new();
    let mut primary_ids = HashSet::new();
    let mut recognized = false;
    let mut primary_table = false;
    for line in body.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if line.starts_with("---") {
            section = line.trim_matches('-').trim();
            columns.clear();
            selected.clear();
            primary_table = false;
            continue;
        }
        let cells: Vec<_> = line.split_whitespace().collect();
        if matches!(cells.first(), Some(&"ID" | &"ChnId")) {
            columns = cells;
            if columns.iter().collect::<HashSet<_>>().len() != columns.len() {
                return Err(malformed());
            }
            selected = fields(name, section)
                .iter()
                .filter_map(|(column, key)| {
                    columns
                        .iter()
                        .position(|candidate| candidate == column)
                        .map(|ix| (ix, *key))
                })
                .collect();
            let required: &[&str] = match name {
                "vdec" => &["TYPE", "Width", "Height"],
                "venc" => &["Type", "Width", "Height"],
                "vpc" => &["input_w", "input_h", "output_w", "output_h"],
                "rc" => &["Gop", "RcMode", "Br(kbps)"],
                _ => &["Width", "Height"],
            };
            primary_table = section == primary && required.iter().all(|key| columns.contains(key));
            recognized |= primary_table;
            table_ids.clear();
            continue;
        }
        if selected.is_empty() {
            continue;
        }
        if cells.len() != columns.len() {
            return Err(malformed());
        }
        let id: u32 = cells[0].parse().map_err(|_| malformed())?;
        if !table_ids.insert(id) {
            return Err(malformed());
        }
        if primary_table {
            primary_ids.insert(id);
        }
        let channel = channels.entry(id).or_insert_with(|| DvppChannel {
            id,
            ..Default::default()
        });
        for (ix, key) in &selected {
            if let Some(value) = value(key, cells[*ix])?
                && channel.fields.insert(key, value).is_some()
            {
                return Err(malformed());
            }
        }
    }
    if !recognized || channels.keys().any(|id| !primary_ids.contains(id)) {
        return Err(malformed());
    }
    Ok(channels.into_values().collect())
}

pub(crate) fn probe(caps: &DeviceCapabilities, tracker: &mut DvppTracker) {
    tracker
        .sources
        .retain(|path, _| caps.ascend && caps.nodes.contains_key(path));
    if !caps.ascend {
        return;
    }
    for path in SOURCES {
        if let Some(status) = caps.nodes.get(*path) {
            let metric = tracker.sources.entry((*path).into()).or_default();
            metric.fail(if *status == MetricStatus::Ready {
                MetricStatus::Loading
            } else {
                status.clone()
            });
        }
    }
}

pub(crate) fn collect(
    output: &str,
    caps: &DeviceCapabilities,
    tracker: &mut DvppTracker,
    now: Instant,
) -> Result<(), MetricStatus> {
    if !caps.ascend {
        tracker.sources.clear();
        return Ok(());
    }
    let parsed = match nodes(output) {
        Ok(nodes) => nodes,
        Err(status) => {
            tracker.fail(status.clone());
            return Err(status);
        }
    };
    if parsed
        .iter()
        .any(|node| !SOURCES.contains(&node.path.as_str()))
    {
        let status = MetricStatus::Failed("unexpected_dvpp_source".into());
        tracker.fail(status.clone());
        return Err(status);
    }
    let mut failure = None;
    tracker
        .sources
        .retain(|path, _| caps.nodes.contains_key(path));
    for path in SOURCES {
        let Some(state) = caps.nodes.get(*path) else {
            continue;
        };
        let result = match parsed.iter().find(|node| node.path == *path) {
            Some(node) if node.status == MetricStatus::Ready => parse(path, &node.body),
            Some(node) => Err(node.status.clone()),
            None if *state == MetricStatus::Ready => Err(MetricStatus::Unavailable),
            None => Err(state.clone()),
        };
        if let Err(status) = &result
            && (*status == MetricStatus::Timeout
                || (failure.is_none() && matches!(status, MetricStatus::Failed(_))))
        {
            failure = Some(status.clone());
        }
        tracker.record(path, result, now);
    }
    failure.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const IDLE: &str = include_str!("../../../tests/fixtures/monitor/ascend-310b-dvpp-idle.txt");
    const ACTIVE: &str = include_str!("../../../tests/fixtures/monitor/ascend-dvpp-simulated.txt");

    fn capabilities() -> DeviceCapabilities {
        DeviceCapabilities {
            ascend: true,
            nodes: SOURCES
                .iter()
                .map(|path| ((*path).into(), MetricStatus::Ready))
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn actual_idle_board_has_readable_sources_and_no_channels() {
        let mut tracker = DvppTracker::default();
        collect(IDLE, &capabilities(), &mut tracker, Instant::now()).unwrap();
        assert_eq!(tracker.sources.len(), 6);
        for metric in tracker.sources.values() {
            assert_eq!(metric.status, MetricStatus::Ready);
            assert!(metric.value.as_ref().unwrap().is_empty());
        }
    }

    #[test]
    fn channels_follow_section_headers_and_keep_rates_distinct() {
        let mut tracker = DvppTracker::default();
        collect(ACTIVE, &capabilities(), &mut tracker, Instant::now()).unwrap();
        let fields = |source: &str| &tracker.sources[source].value.as_ref().unwrap()[0].fields;
        let encoder = fields("/proc/umap/venc");
        assert_eq!(encoder["width"], "3840");
        assert_eq!(encoder["source_fps"], "30");
        assert_eq!(encoder["target_fps"], "25");
        assert_eq!(encoder["driver_fps"], "24.75");
        assert_eq!(encoder["pid"], "2200");
        assert_eq!(fields("/proc/umap/vdec")["pid"], "2100");
        assert_eq!(fields("/proc/umap/vdec")["decode_fps"], "24.5");
        assert_eq!(fields("/proc/umap/rc")["target_fps"], "25/1");
        assert_eq!(fields("/proc/umap/rc")["instant_kbps"], "7530");
        assert_eq!(fields("/proc/umap/h264e")["encode_succeeded"], "998");
        assert_eq!(fields("/proc/umap/vpc")["output_width"], "640");
    }

    #[test]
    fn rejects_truncation_bad_numbers_duplicates_and_orphan_channels() {
        let node = nodes(ACTIVE)
            .unwrap()
            .into_iter()
            .find(|n| n.path.ends_with("/vdec"))
            .unwrap();
        for body in [
            String::new(),
            node.body
                .replace("3 H265 1920 1080 2097152 8", "3 H265 1920"),
            node.body.replace("3 24.5 25 8192", "3 NaN 25 8192"),
            node.body.replace("3 24.5 25 8192", "3 -1 25 8192"),
            node.body.replace("3 24.5 25 8192", "3 25/0 25 8192"),
            node.body
                .replace("3 24.5 25 8192", "3 24.5 25 8192\n3 24.5 25 8192"),
            node.body.replace("3 12 0 2100", "4 12 0 2100"),
            "x".repeat(262145),
        ] {
            assert!(parse(&node.path, &body).is_err());
        }
        let missing = parse(
            &node.path,
            &node.body.replace("3 24.5 25 8192", "3 N/A 25 8192"),
        )
        .unwrap();
        assert!(!missing[0].fields.contains_key("decode_fps"));
        assert_eq!(missing[0].fields["send_fps"], "25");
    }

    #[test]
    fn partial_failures_preserve_each_source_and_idle_clears_old_channels() {
        let now = Instant::now();
        let caps = capabilities();
        let mut tracker = DvppTracker::default();
        collect(ACTIVE, &caps, &mut tracker, now).unwrap();
        let broken = ACTIVE.replace(
            "@@node /proc/umap/vdec ready",
            "@@node /proc/umap/vdec timeout",
        );
        assert_eq!(
            collect(&broken, &caps, &mut tracker, now + Duration::from_secs(5)),
            Err(MetricStatus::Timeout)
        );
        let decoder = &tracker.sources["/proc/umap/vdec"];
        assert!(decoder.stale);
        assert_eq!(decoder.observed, Some(now));
        assert_eq!(decoder.value.as_ref().unwrap().len(), 1);
        assert!(!tracker.sources["/proc/umap/venc"].stale);
        collect(IDLE, &caps, &mut tracker, now + Duration::from_secs(10)).unwrap();
        assert!(
            tracker
                .sources
                .values()
                .all(|metric| !metric.stale && metric.value.as_ref().unwrap().is_empty())
        );
        assert!(collect("@@node /proc/umap/vdec ready", &caps, &mut tracker, now).is_err());
        assert!(tracker.sources.values().all(|metric| metric.stale));
    }

    #[test]
    fn permission_missing_sources_and_non_ascend_hosts_are_distinct() {
        let mut caps = capabilities();
        caps.nodes
            .insert(SOURCES[0].into(), MetricStatus::PermissionDenied);
        caps.nodes
            .insert(SOURCES[1].into(), MetricStatus::Unsupported);
        let mut tracker = DvppTracker::default();
        collect("@@complete", &caps, &mut tracker, Instant::now()).unwrap();
        assert_eq!(
            tracker.sources[SOURCES[0]].status,
            MetricStatus::PermissionDenied
        );
        assert_eq!(
            tracker.sources[SOURCES[1]].status,
            MetricStatus::Unsupported
        );
        assert_eq!(
            tracker.sources[SOURCES[2]].status,
            MetricStatus::Unavailable
        );
        caps.ascend = false;
        collect(ACTIVE, &caps, &mut tracker, Instant::now()).unwrap();
        assert!(tracker.sources.is_empty());
        assert!(
            super::super::command(&caps, crate::monitor::CollectorDomain::Dvpp, true).is_none()
        );
        assert!(
            SOURCES
                .iter()
                .all(|path| !super::super::SUDO_NODES.contains(path))
        );
    }
    #[test]
    fn probe_uses_fixed_nodes_and_reconnect_marks_old_values_stale() {
        let mut text =
            "@@node /proc/device-tree/compatible ready\nhisilicon,ascend610\n@@endnode\n"
                .to_owned();
        for path in SOURCES {
            text.push_str(&format!("@@node {path} ready\n@@endnode\n"));
        }
        text.push_str("@@complete");
        let caps = super::super::parse_probe(&text).unwrap();
        assert!(caps.ascend);
        assert!(
            super::super::command(&caps, crate::monitor::CollectorDomain::Dvpp, true).is_some()
        );
        let mut tracker = DvppTracker::default();
        collect(ACTIVE, &caps, &mut tracker, Instant::now()).unwrap();
        probe(&caps, &mut tracker);
        assert!(
            tracker
                .sources
                .values()
                .all(|metric| metric.stale && metric.status == MetricStatus::Loading)
        );
        tracker.fail(MetricStatus::Timeout);
        assert!(
            tracker
                .sources
                .values()
                .all(|metric| metric.stale && metric.status == MetricStatus::Timeout)
        );
        assert!(
            collect(
                "@@node /proc/pressure/cpu ready\n@@endnode\n@@complete",
                &caps,
                &mut tracker,
                Instant::now()
            )
            .is_err()
        );
        probe(&DeviceCapabilities::default(), &mut tracker);
        assert!(tracker.sources.is_empty());
    }
}
