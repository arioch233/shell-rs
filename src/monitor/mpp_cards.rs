//! MPP 卡片的会话吞吐、时钟与任务读数，诊断字段沿用可复制详情行

use super::{
    mpp::{CodecKind, MppReading, MppSession, MppTracker},
    snapshot::{Metric, MetricStatus},
};
use crate::{
    i18n::t,
    shared::{compact_detail_row, format_bytes, numeric, stacked_detail_row},
};
use gpui_kit::base::SelectableText;
use gpui_kit::component::{
    ActiveTheme as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

fn kind_label(kind: CodecKind) -> SharedString {
    match kind {
        CodecKind::Encoder => t!("monitor.mpp.encoder"),
        CodecKind::Decoder => t!("monitor.mpp.decoder"),
        CodecKind::Other => t!("monitor.mpp.other_codec"),
    }
}

fn core_label(core: &str) -> SharedString {
    if let Some(index) = core.strip_prefix("rkvenc-core") {
        t!("monitor.mpp.encoder_core", index = index)
    } else if let Some(index) = core.strip_prefix("rkvdec-core") {
        t!("monitor.mpp.decoder_core", index = index)
    } else {
        core.to_owned().into()
    }
}

fn session_title(session: &MppSession) -> SharedString {
    t!(
        "monitor.mpp.session_title",
        kind = kind_label(session.kind),
        index = session.index,
        pid = session.pid
    )
}

fn source_label(source: &str) -> SharedString {
    match source.rsplit('/').next().unwrap_or(source) {
        "version" => t!("monitor.hardware.driver_info"),
        "supports-device" => t!("monitor.mpp.supports"),
        "timing_en" => t!("monitor.mpp.timing"),
        "load_interval" => t!("monitor.mpp.load_interval"),
        "sessions-summary" | "sessions_summary" => t!("monitor.mpp.sessions"),
        "clk_summary" => t!("monitor.mpp.clocks"),
        "task_count" => format!(
            "{} · {}",
            t!("monitor.mpp.tasks"),
            core_label(source.split('/').nth(3).unwrap_or(source))
        )
        .into(),
        "sessions-info" => format!(
            "{} · {}",
            t!("monitor.mpp.diagnostics"),
            core_label(source.split('/').nth(3).unwrap_or(source))
        )
        .into(),
        _ => source.to_owned().into(),
    }
}

fn source_state(source: &str, metric: &Metric<MppReading>, cx: &App) -> AnyElement {
    super::monitor_panel::status_row(
        &format!("mpp-{source}"),
        &metric.status,
        metric.stale.then_some(metric.observed).flatten(),
        cx,
    )
    .into_any_element()
}

fn session_details(session: &MppSession, cx: &App) -> AnyElement {
    let mut fields = vec![(
        "device",
        t!("monitor.hardware.device_node"),
        session.device.clone(),
    )];
    if let Some(memory) = session.memory_bytes {
        fields.push(("memory", t!("monitor.mpp.memory"), format_bytes(memory)));
    }
    for (key, label) in [
        ("fps_in", t!("monitor.mpp.fps_in")),
        ("fps_out", t!("monitor.mpp.fps_out")),
        ("rc_mode", t!("monitor.mpp.rc_mode")),
        ("bitrate", t!("monitor.mpp.bitrate")),
        ("gop_size", "GOP".into()),
        ("profile", "Profile".into()),
    ] {
        if let Some(value) = session.fields.get(key) {
            let value = if key == "bitrate" {
                value
                    .parse::<f64>()
                    .map(|rate| format!("{:.2} Mbps", rate / 1_000_000.))
                    .unwrap_or_else(|_| value.clone())
            } else if key == "rc_mode" {
                value.to_uppercase()
            } else {
                value.clone()
            };
            fields.push((key, label, value));
        }
    }
    let value = fields
        .iter()
        .map(|(_, label, value)| format!("{label}: {value}"))
        .collect::<Vec<_>>()
        .join("\n");
    let id = SharedString::from(format!(
        "mpp-session-detail-{}-{}",
        session.pid, session.index
    ));
    v_flex()
        .id(id.clone())
        .test_support()
        .aria_label(value)
        .min_w_0()
        .gap_2()
        .pt_3()
        .border_t_1()
        .border_color(cx.theme().border)
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .child(session_title(session)),
        )
        .children(fields.into_iter().map(|(key, label, value)| {
            let field_id = ElementId::from((ElementId::from(id.clone()), key));
            if key == "device" {
                stacked_detail_row(field_id, label, value.into(), cx).into_any_element()
            } else {
                compact_detail_row(field_id, label, value.into(), cx).into_any_element()
            }
        }))
        .into_any_element()
}

pub(super) fn render(
    tracker: &MppTracker,
    expanded: bool,
    all_sessions: bool,
    dispatch: &FocusHandle,
    cx: &App,
) -> impl IntoElement {
    let mut children = Vec::new();
    let mut tasks = Vec::new();
    let session_limit = if all_sessions { usize::MAX } else { 32 };
    if let Some((source, metric)) = tracker.sessions() {
        if !expanded {
            if metric.stale || metric.status != MetricStatus::Ready {
                children.push(source_state(source, metric, cx));
            }
            if let Some(MppReading::Sessions(sessions)) = &metric.value {
                let encoders = sessions
                    .iter()
                    .filter(|session| session.kind == CodecKind::Encoder)
                    .count();
                let decoders = sessions
                    .iter()
                    .filter(|session| session.kind == CodecKind::Decoder)
                    .count();
                let fps: Vec<_> = sessions
                    .iter()
                    .filter(|session| session.kind == CodecKind::Encoder)
                    .filter_map(|session| session.fps)
                    .collect();
                children.push(
                    div()
                        .id("mpp-session-count")
                        .test_support()
                        .aria_label(t!(
                            "monitor.mpp.counts",
                            total = sessions.len(),
                            encoders = encoders,
                            decoders = decoders
                        ))
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(t!(
                            "monitor.mpp.counts",
                            total = sessions.len(),
                            encoders = encoders,
                            decoders = decoders
                        ))
                        .into_any_element(),
                );
                let sum = if fps.is_empty() || metric.stale {
                    "—".to_owned()
                } else {
                    format!("{:.2} FPS", fps.iter().sum::<f64>())
                };
                children.push(
                    compact_detail_row(
                        "mpp-fps-sum".into(),
                        t!("monitor.mpp.fps_sum"),
                        sum.into(),
                        cx,
                    )
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .into_any_element(),
                );
                if !fps.is_empty() && fps.len() != encoders {
                    children.push(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(t!(
                                "monitor.mpp.partial_fps",
                                known = fps.len(),
                                total = encoders
                            ))
                            .into_any_element(),
                    );
                }
                if sessions.len() > 32 {
                    let dispatch = dispatch.clone();
                    children.push(
                        Button::new("mpp-all-sessions")
                            .ghost()
                            .small()
                            .w_full()
                            .label(if all_sessions {
                                t!("monitor.mpp.fewer_sessions")
                            } else {
                                t!("monitor.mpp.more_sessions", count = sessions.len())
                            })
                            .on_click(move |_, window, cx| {
                                dispatch.dispatch_action(
                                    &crate::app::ToggleMonitorMetric("mpp/sessions/all".into()),
                                    window,
                                    cx,
                                )
                            })
                            .into_any_element(),
                    );
                }
                if !sessions.is_empty() {
                    children.push(
                        h_flex()
                            .gap_3()
                            .min_w_0()
                            .pt_3()
                            .border_t_1()
                            .border_color(cx.theme().border)
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(div().flex_1().min_w_0().child(t!("monitor.mpp.sessions")))
                            .child(t!("monitor.mpp.actual_fps_column"))
                            .into_any_element(),
                    );
                }
                for session in sessions.iter().take(session_limit) {
                    let codec = session.fields.get("format").map(|s| {
                        match s.to_ascii_lowercase().as_str() {
                            "h264" | "avc" => "H.264".into(),
                            "h265" | "hevc" => "H.265".into(),
                            _ => s.to_uppercase(),
                        }
                    });
                    let resolution = session
                        .fields
                        .get("width")
                        .zip(session.fields.get("height"))
                        .map(|(w, h)| format!("{w} × {h}"));
                    let mut lines: Vec<_> = codec.into_iter().chain(resolution).collect();
                    if session.kind == CodecKind::Encoder {
                        let fps = session
                            .fps
                            .map(|fps| format!("{fps:.2} FPS"))
                            .unwrap_or_else(|| "—".into());
                        lines.push(format!("{}: {fps}", t!("monitor.mpp.actual_fps")));
                    }
                    let id = SharedString::from(format!(
                        "mpp-session-{}-{}",
                        session.pid, session.index
                    ));
                    let stream = lines
                        .iter()
                        .take(if session.kind == CodecKind::Encoder {
                            lines.len().saturating_sub(1)
                        } else {
                            lines.len()
                        })
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(" · ");
                    let fps = if session.kind == CodecKind::Encoder {
                        session.fps
                    } else {
                        None
                    };
                    children.push(
                        v_flex()
                            .id(id.clone())
                            .test_support()
                            .aria_label(lines.join(" · "))
                            .min_w_0()
                            .gap_1()
                            .child(
                                h_flex()
                                    .gap_3()
                                    .min_w_0()
                                    .child(div().flex_1().min_w_0().text_sm().child(t!(
                                        "monitor.mpp.session_heading",
                                        kind = kind_label(session.kind),
                                        index = session.index
                                    )))
                                    .child(
                                        numeric(
                                            fps.map(|fps| format!("{fps:.2}"))
                                                .unwrap_or_else(|| "—".into()),
                                            cx,
                                        )
                                        .id((ElementId::from(id), "fps"))
                                        .test_support()
                                        .text_sm()
                                        .flex_shrink_0(),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .flex_wrap()
                                    .gap_x_2()
                                    .gap_y_1()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .when(!stream.is_empty(), |row| row.child(stream))
                                    .child(format!("PID {}", session.pid)),
                            )
                            .into_any_element(),
                    );
                }
            }
        } else if let Some(MppReading::Sessions(sessions)) = &metric.value {
            children.extend(
                sessions
                    .iter()
                    .take(session_limit)
                    .map(|session| session_details(session, cx)),
            );
        }
    }
    for (source, metric) in &tracker.sources {
        if !expanded {
            match &metric.value {
                Some(MppReading::Clocks(clocks)) => {
                    let mut group = v_flex()
                        .min_w_0()
                        .gap_2()
                        .pt_3()
                        .border_t_1()
                        .border_color(cx.theme().border)
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .child(t!("monitor.mpp.clocks")),
                        );
                    if metric.stale || metric.status != MetricStatus::Ready {
                        group = group.child(source_state(source, metric, cx));
                    }
                    if !clocks.iter().any(|clock| clock.name.starts_with("clk_")) {
                        group = group.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(t!("monitor.mpp.no_core_clocks")),
                        );
                    }
                    for clock in clocks.iter().filter(|clock| clock.name.starts_with("clk_")) {
                        let label = core_label(&clock.core);
                        let value = format!("{:.0} MHz", clock.rate_hz as f64 / 1_000_000.);
                        group = group.child(
                            compact_detail_row(
                                SharedString::from(format!("mpp-clock-{}", clock.name)).into(),
                                label.clone(),
                                value.clone().into(),
                                cx,
                            )
                            .aria_label(format!("{label}: {value}"))
                            .text_sm(),
                        );
                    }
                    children.push(group.into_any_element());
                }
                Some(MppReading::Tasks { index, per_second }) => {
                    let core = source.split('/').nth(3).unwrap_or(source);
                    let rate = if metric.stale { None } else { *per_second };
                    let value = t!(
                        "monitor.mpp.task_value",
                        index = index,
                        rate = rate
                            .map(|rate| format!("{rate:.2}"))
                            .unwrap_or_else(|| "—".into())
                    );
                    let rate = rate
                        .map(|rate| format!("{rate:.2}"))
                        .unwrap_or_else(|| "—".into());
                    tasks.push(
                        v_flex()
                            .id(SharedString::from(format!("mpp-task-{core}")))
                            .test_support()
                            .aria_label(value)
                            .min_w_0()
                            .gap_1()
                            .child(
                                compact_detail_row(
                                    SharedString::from(format!("mpp-task-rate-{core}")).into(),
                                    core_label(core),
                                    t!("monitor.mpp.task_rate", rate = rate),
                                    cx,
                                )
                                .text_sm(),
                            )
                            .child(compact_detail_row(
                                SharedString::from(format!("mpp-task-index-{core}")).into(),
                                t!("monitor.mpp.task_index"),
                                index.to_string().into(),
                                cx,
                            ))
                            .into_any_element(),
                    );
                    if metric.stale || metric.status != MetricStatus::Ready {
                        tasks.push(source_state(source, metric, cx));
                    }
                }
                None if metric.status != MetricStatus::Unsupported
                    && (source.ends_with("/task_count") || source.ends_with("/clk_summary")) =>
                {
                    children.push(
                        div()
                            .text_xs()
                            .child(source_label(source))
                            .into_any_element(),
                    );
                    children.push(source_state(source, metric, cx));
                }
                _ => {}
            }
            continue;
        }
        // 缺失别名不重复出现，真实失败与权限状态仍保留
        if metric.status == MetricStatus::Unsupported {
            continue;
        }
        let mut values = Vec::<String>::new();
        match &metric.value {
            Some(MppReading::Clocks(clocks)) => {
                values.extend(clocks.iter().map(|clock| {
                    t!(
                        "monitor.mpp.clock_detail",
                        name = &clock.name,
                        rate = format!("{:.0}", clock.rate_hz as f64 / 1_000_000.),
                        enabled = clock.enable_count
                    )
                    .to_string()
                }));
            }
            Some(MppReading::Diagnostic { bytes, preview }) => {
                values.push(t!("monitor.mpp.preview", bytes = bytes).to_string());
                values.push(preview.clone());
            }
            Some(MppReading::Text(text)) => {
                values.push(if source.ends_with("/timing_en") {
                    match text.as_str() {
                        "0" => t!("monitor.mpp.timing_off").to_string(),
                        "1" => t!("monitor.mpp.timing_on").to_string(),
                        _ => text.clone(),
                    }
                } else {
                    text.clone()
                });
            }
            _ => {}
        }
        let mut section = v_flex()
            .min_w_0()
            .gap_2()
            .pt_2()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .child(source_label(source)),
            );
        if metric.stale || metric.status != MetricStatus::Ready {
            section = section.child(source_state(source, metric, cx));
        }
        if let Some(at) = metric.observed {
            section = section.child(
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
        if !values.is_empty() {
            let id = SharedString::from(format!("mpp-detail-{source}"));
            let value = values.join("\n");
            let mut reading = v_flex()
                .id(id.clone())
                .test_support()
                .aria_label(value.clone())
                .min_w_0()
                .gap_2();
            if let Some(MppReading::Clocks(clocks)) = &metric.value {
                let mut cores = std::collections::BTreeMap::<&str, Vec<_>>::new();
                for clock in clocks {
                    cores.entry(&clock.core).or_default().push(clock);
                }
                for (core, clocks) in cores {
                    let mut group = v_flex().min_w_0().gap_2().child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .child(core_label(core)),
                    );
                    for clock in clocks {
                        let role = if clock.name.starts_with("aclk_") {
                            t!("monitor.mpp.bus_clock")
                        } else if clock.name.starts_with("hclk_") {
                            t!("monitor.mpp.peripheral_clock")
                        } else {
                            t!("monitor.mpp.core_clock")
                        };
                        group = group.child(
                            v_flex()
                                .min_w_0()
                                .gap_1()
                                .child(compact_detail_row(
                                    SharedString::from(format!("mpp-clock-detail-{}", clock.name))
                                        .into(),
                                    role,
                                    format!("{:.0} MHz", clock.rate_hz as f64 / 1_000_000.).into(),
                                    cx,
                                ))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(t!(
                                            "monitor.mpp.clock_identity",
                                            name = &clock.name,
                                            count = clock.enable_count
                                        )),
                                ),
                        );
                    }
                    reading = reading.child(group);
                }
                reading = reading.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(t!("monitor.mpp.clock_note")),
                );
            } else {
                reading = reading.child(
                    div().text_xs().min_w_0().whitespace_normal().child(
                        SelectableText::new((ElementId::from(id), "value"), value)
                            .selection_color(cx.theme().selection),
                    ),
                );
            }
            section = section.child(reading);
        }
        if matches!(metric.value, Some(MppReading::Tasks { .. })) {
            section = section.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("monitor.mpp.task_note")),
            );
        }
        section = section.child(stacked_detail_row(
            SharedString::from(format!("mpp-source-{source}")).into(),
            t!("monitor.hardware.source"),
            source.clone().into(),
            cx,
        ));
        children.push(section.into_any_element());
    }
    if !tasks.is_empty() {
        children.push(
            v_flex()
                .min_w_0()
                .gap_2()
                .pt_3()
                .border_t_1()
                .border_color(cx.theme().border)
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .child(t!("monitor.mpp.tasks")),
                )
                .children(tasks)
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(t!("monitor.mpp.task_rate_note")),
                )
                .into_any_element(),
        );
    }
    v_flex().min_w_0().gap_3().children(children)
}
