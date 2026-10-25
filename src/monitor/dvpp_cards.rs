//! HiDVPP 通道概览与按来源展开的诊断字段

use super::{
    dvpp::{DvppChannel, DvppTracker},
    monitor_panel::status_row,
    snapshot::MetricStatus,
};
use crate::{
    app::{CatalogIcon, ToggleMonitorMetric},
    i18n::t,
    shared::{card, compact_detail_row, stacked_detail_row},
};
use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable as _,
    button::{Button, ButtonVariants as _},
    v_flex,
};
use gpui_kit::{prelude::FluentBuilder as _, *};
use std::collections::HashSet;

fn source_title(name: &str) -> SharedString {
    match name {
        "vdec" => t!("monitor.dvpp.decoder"),
        "venc" => t!("monitor.dvpp.encoder"),
        "vpc" => t!("monitor.dvpp.image"),
        "rc" => t!("monitor.dvpp.rate_control"),
        "h264e" => "H.264".into(),
        "h265e" => "H.265".into(),
        _ => name.to_owned().into(),
    }
}

fn field_label(key: &str) -> SharedString {
    match key {
        "codec" => t!("monitor.dvpp.field.codec"),
        "width" => t!("monitor.dvpp.field.width"),
        "height" => t!("monitor.dvpp.field.height"),
        "state" => t!("monitor.dvpp.field.state"),
        "stream_buffer" => t!("monitor.dvpp.field.stream_buffer"),
        "frame_buffers" => t!("monitor.dvpp.field.frame_buffers"),
        "decode_fps" => t!("monitor.dvpp.field.decode_fps"),
        "send_fps" => t!("monitor.dvpp.field.send_fps"),
        "queued_bytes" => t!("monitor.dvpp.field.queued_bytes"),
        "protocol_errors" => t!("monitor.dvpp.field.protocol_errors"),
        "stream_errors" => t!("monitor.dvpp.field.stream_errors"),
        "size_errors" => t!("monitor.dvpp.field.size_errors"),
        "submitted" => t!("monitor.dvpp.field.submitted"),
        "decoded" => t!("monitor.dvpp.field.decoded"),
        "retrieved" => t!("monitor.dvpp.field.retrieved"),
        "released" => t!("monitor.dvpp.field.released"),
        "discarded" => t!("monitor.dvpp.field.discarded"),
        "queued_frames" => t!("monitor.dvpp.field.queued_frames"),
        "busy_buffers" => t!("monitor.dvpp.field.busy_buffers"),
        "average_task_us" => t!("monitor.dvpp.field.average_task_us"),
        "maximum_task_us" => t!("monitor.dvpp.field.maximum_task_us"),
        "minimum_task_us" => t!("monitor.dvpp.field.minimum_task_us"),
        "pid" => t!("monitor.dvpp.field.pid"),
        "user_channel" => t!("monitor.dvpp.field.user_channel"),
        "vfid" => t!("monitor.dvpp.field.vfid"),
        "source_fps" => t!("monitor.dvpp.field.source_fps"),
        "target_fps" => t!("monitor.dvpp.field.target_fps"),
        "driver_fps" => t!("monitor.dvpp.field.driver_fps"),
        "overloads" => t!("monitor.dvpp.field.overloads"),
        "ring_skips" => t!("monitor.dvpp.field.ring_skips"),
        "rc_skips" => t!("monitor.dvpp.field.rc_skips"),
        "encode_started" => t!("monitor.dvpp.field.encode_started"),
        "encode_succeeded" => t!("monitor.dvpp.field.encode_succeeded"),
        "lost" => t!("monitor.dvpp.field.lost"),
        "reencoded" => t!("monitor.dvpp.field.reencoded"),
        "gop" => t!("monitor.dvpp.field.gop"),
        "rc_mode" => t!("monitor.dvpp.field.rc_mode"),
        "configured_kbps" => t!("monitor.dvpp.field.configured_kbps"),
        "instant_kbps" => t!("monitor.dvpp.field.instant_kbps"),
        "rc_fps" => t!("monitor.dvpp.field.rc_fps"),
        "output_width" => t!("monitor.dvpp.field.output_width"),
        "output_height" => t!("monitor.dvpp.field.output_height"),
        "function" => t!("monitor.dvpp.field.function"),
        "operations" => t!("monitor.dvpp.field.operations"),
        "memory_bytes" => t!("monitor.dvpp.field.memory_bytes"),
        "working_queue" => t!("monitor.dvpp.field.working_queue"),
        "event_queue" => t!("monitor.dvpp.field.event_queue"),
        "results" => t!("monitor.dvpp.field.results"),
        "interrupt_errors" => t!("monitor.dvpp.field.interrupt_errors"),
        "retries" => t!("monitor.dvpp.field.retries"),
        _ => key.to_owned().into(),
    }
}

fn formatted(key: &str, value: &str) -> String {
    if key.ends_with("_kbps") {
        return value
            .parse::<f64>()
            .map(|n| format!("{:.2} Mbps", n / 1000.))
            .unwrap_or_else(|_| value.into());
    }
    if key.ends_with("_us") {
        return format!("{value} µs");
    }
    if key.ends_with("fps") {
        return format!("{value} FPS");
    }
    if key == "codec" {
        return match value.to_ascii_lowercase().as_str() {
            "h264" | "avc" => "H.264".into(),
            "h265" | "hevc" => "H.265".into(),
            "jpeg" | "mjpeg" => value.to_uppercase(),
            _ => value.into(),
        };
    }
    value.into()
}

fn channel_rows(channel: &DvppChannel, name: &str, expanded: bool, cx: &App) -> AnyElement {
    let id = format!("dvpp-{name}-{}", channel.id);
    let mut content = v_flex()
        .id(SharedString::from(id.clone()))
        .test_support()
        .min_w_0()
        .gap_2()
        .pt_3()
        .border_t_1()
        .border_color(cx.theme().border)
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .child(t!("monitor.dvpp.channel", id = channel.id)),
        );
    let fields = &channel.fields;
    if !expanded {
        let mut identity = Vec::new();
        if let Some(codec) = fields.get("codec") {
            identity.push(formatted("codec", codec));
        }
        if let Some((width, height)) = fields.get("width").zip(fields.get("height")) {
            identity.push(format!("{width} × {height}"));
        }
        if let Some((width, height)) = fields.get("output_width").zip(fields.get("output_height")) {
            identity.push(format!("→ {width} × {height}"));
        }
        if let Some(pid) = fields.get("pid") {
            identity.push(format!("PID {pid}"));
        }
        let identity = identity.join(" · ");
        content = content.aria_label(identity.clone()).child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(identity),
        );
        let fps = match name {
            "vdec" => "decode_fps",
            "vpc" => "send_fps",
            _ => "driver_fps",
        };
        content = content.child(compact_detail_row(
            SharedString::from(format!("{id}-{fps}")).into(),
            field_label(fps),
            fields
                .get(fps)
                .map(|v| formatted(fps, v))
                .unwrap_or_else(|| "—".into())
                .into(),
            cx,
        ));
    } else {
        for (group, keys) in [
            (
                "monitor.dvpp.group.identity",
                &["codec", "pid", "user_channel", "vfid", "state", "function"][..],
            ),
            (
                "monitor.dvpp.group.configuration",
                &[
                    "width",
                    "height",
                    "output_width",
                    "output_height",
                    "source_fps",
                    "target_fps",
                    "rc_mode",
                    "configured_kbps",
                    "gop",
                ][..],
            ),
            (
                "monitor.dvpp.group.throughput",
                &[
                    "decode_fps",
                    "driver_fps",
                    "send_fps",
                    "rc_fps",
                    "instant_kbps",
                    "submitted",
                    "decoded",
                    "retrieved",
                    "released",
                    "encode_started",
                    "encode_succeeded",
                    "results",
                    "operations",
                ][..],
            ),
            (
                "monitor.dvpp.group.queues",
                &[
                    "queued_frames",
                    "queued_bytes",
                    "busy_buffers",
                    "frame_buffers",
                    "stream_buffer",
                    "working_queue",
                    "event_queue",
                    "memory_bytes",
                ][..],
            ),
            (
                "monitor.dvpp.group.errors",
                &[
                    "protocol_errors",
                    "stream_errors",
                    "size_errors",
                    "discarded",
                    "lost",
                    "reencoded",
                    "overloads",
                    "ring_skips",
                    "rc_skips",
                    "interrupt_errors",
                    "retries",
                ][..],
            ),
            (
                "monitor.dvpp.group.timings",
                &["average_task_us", "maximum_task_us", "minimum_task_us"][..],
            ),
        ] {
            if !keys.iter().any(|key| fields.contains_key(key)) {
                continue;
            }
            content = content.child(
                div()
                    .pt_2()
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .child(t!(group)),
            );
            for key in keys {
                if let Some(value) = fields.get(key) {
                    content = content.child(compact_detail_row(
                        SharedString::from(format!("{id}-detail-{key}")).into(),
                        field_label(key),
                        formatted(key, value).into(),
                        cx,
                    ));
                }
            }
        }
    }
    content.into_any_element()
}

pub(super) fn render(
    tracker: &DvppTracker,
    details: &HashSet<String>,
    dispatch: &FocusHandle,
    cx: &App,
) -> Option<AnyElement> {
    if !tracker
        .sources
        .values()
        .any(|metric| metric.status != MetricStatus::Unsupported)
    {
        return None;
    }
    let expanded = details.contains("hardware/dvpp");
    let all = details.contains("dvpp/channels/all");
    let limit = if all { usize::MAX } else { 8 };
    let mut content = card("monitor-dvpp", CatalogIcon::Layers, "DVPP", None, cx).child(
        div()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(t!("monitor.dvpp.subtitle")),
    );
    let more = tracker.sources.iter().any(|(source, metric)| {
        (expanded || matches!(source.rsplit('/').next(), Some("vdec" | "venc" | "vpc")))
            && metric
                .value
                .as_ref()
                .is_some_and(|channels| channels.len() > 8)
    });
    if more {
        let dispatch = dispatch.clone();
        content = content.child(
            Button::new("dvpp-all-channels")
                .ghost()
                .small()
                .w_full()
                .label(if all {
                    t!("monitor.dvpp.fewer")
                } else {
                    t!("monitor.dvpp.more")
                })
                .on_click(move |_, window, cx| {
                    dispatch.dispatch_action(
                        &ToggleMonitorMetric("dvpp/channels/all".into()),
                        window,
                        cx,
                    )
                }),
        );
    }

    for name in ["vdec", "venc", "vpc", "rc", "h264e", "h265e"] {
        if !expanded && matches!(name, "rc" | "h264e" | "h265e") {
            continue;
        }
        let source = format!("/proc/umap/{name}");
        let Some(metric) = tracker.sources.get(&source) else {
            continue;
        };
        if metric.status == MetricStatus::Unsupported {
            continue;
        }
        let channels = metric.value.as_deref().unwrap_or_default();
        let current = metric.status == MetricStatus::Ready && !metric.stale;
        let count = if current {
            channels.len().to_string()
        } else {
            "—".into()
        };
        let mut group = v_flex().min_w_0().gap_2().child(
            compact_detail_row(
                SharedString::from(format!("dvpp-{name}-count")).into(),
                source_title(name),
                t!("monitor.dvpp.count", count = count),
                cx,
            )
            .text_sm(),
        );
        if !current {
            group = group.child(status_row(
                &format!("dvpp-{name}"),
                &metric.status,
                metric.stale.then_some(metric.observed).flatten(),
                cx,
            ));
        } else if channels.is_empty() {
            group = group.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("monitor.dvpp.idle")),
            );
        }
        // 历史值只放在详情里，避免把旧帧率当作实时读数
        if current || expanded {
            for channel in channels.iter().take(limit) {
                group = group.child(channel_rows(channel, name, expanded, cx));
                if !expanded && name == "venc" {
                    let rc = tracker
                        .sources
                        .get("/proc/umap/rc")
                        .filter(|rc| {
                            rc.status == MetricStatus::Ready
                                && !rc.stale
                                && rc.observed == metric.observed
                        })
                        .and_then(|rc| rc.value.as_ref())
                        .and_then(|rows| rows.iter().find(|row| row.id == channel.id));
                    let bitrate = rc.and_then(|channel| channel.fields.get("instant_kbps"));
                    group = group.child(compact_detail_row(
                        SharedString::from(format!("dvpp-venc-{}-bitrate", channel.id)).into(),
                        t!("monitor.dvpp.field.instant_kbps"),
                        bitrate
                            .map(|v| formatted("instant_kbps", v))
                            .unwrap_or_else(|| "—".into())
                            .into(),
                        cx,
                    ));
                }
            }
        }
        if expanded {
            group = group.child(stacked_detail_row(
                SharedString::from(format!("dvpp-{name}-source")).into(),
                t!("monitor.hardware.source"),
                source.into(),
                cx,
            ));
            if let Some(at) = metric.observed {
                group = group.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(t!(
                            "monitor.hardware.observed",
                            seconds = cx
                                .background_executor()
                                .now()
                                .saturating_duration_since(at)
                                .as_secs()
                        )),
                );
            }
        }
        content = content.child(group);
    }
    let dispatch = dispatch.clone();
    content = content
        .child(
            Button::new("monitor-details-dvpp")
                .ghost()
                .small()
                .w_full()
                .justify_between()
                .label(t!("monitor.hardware.details"))
                .toggled(expanded)
                .child(
                    Icon::new(if expanded {
                        CatalogIcon::ChevronUp
                    } else {
                        CatalogIcon::ChevronDown
                    })
                    .xsmall(),
                )
                .on_click(move |_, window, cx| {
                    dispatch.dispatch_action(
                        &ToggleMonitorMetric("hardware/dvpp".into()),
                        window,
                        cx,
                    )
                }),
        )
        .when(expanded, |view| {
            view.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("monitor.dvpp.note")),
            )
        });
    Some(content.into_any_element())
}
