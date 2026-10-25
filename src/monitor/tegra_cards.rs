//! Jetson 媒体引擎卡片，运行状态、硬件负载与电源驻留比例分开呈现

use super::{
    collectors::tegra::{ENGINES, FIELDS, source},
    hardware_cards::tegra_engine,
    monitor_panel::{status_row, status_text},
    snapshot::MetricStatus,
    tegra::TegraTracker,
};
use crate::{
    app::{CatalogIcon, ConfigureMonitorAccess, ToggleMonitorMetric},
    i18n::t,
    shared::{card, compact_detail_row, meter, stacked_detail_row},
};
use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable as _,
    button::{Button, ButtonVariants as _},
    v_flex,
};
use gpui_kit::*;
use std::collections::HashSet;

fn runtime_label(value: &str) -> SharedString {
    match value {
        "active" => t!("monitor.jetson.awake"),
        "suspended" => t!("monitor.jetson.suspended"),
        "suspending" => t!("monitor.jetson.suspending"),
        "resuming" => t!("monitor.jetson.resuming"),
        "unsupported" => t!("monitor.state.unsupported"),
        _ => t!("monitor.state.failed"),
    }
}

fn field_label(field: &str) -> SharedString {
    match field {
        "usage" => t!("monitor.jetson.load"),
        "cur_freq" => t!("monitor.hardware.frequency"),
        "governor" => t!("monitor.jetson.governor"),
        "device/power/runtime_status" => t!("monitor.jetson.runtime"),
        "device/power/runtime_active_time" => t!("monitor.jetson.active_time"),
        _ => t!("monitor.jetson.suspended_time"),
    }
}

pub(super) fn render(
    tracker: &TegraTracker,
    details: &HashSet<String>,
    dispatch: &FocusHandle,
    cx: &App,
) -> Vec<AnyElement> {
    let mut cards = Vec::new();
    for engine in ENGINES {
        let Some(state) = tracker.engines.get(engine.device) else {
            continue;
        };
        let id = format!("tegra-{}", engine.device);
        let key = format!("hardware/tegra/{}", engine.device);
        let expanded = details.contains(&key);
        let title = tegra_engine(engine.device).unwrap().0;
        let runtime = state.current("device/power/runtime_status");
        let usage = state.sources.get("usage");
        let load = state
            .current("usage")
            .and_then(|s| s.parse::<f32>().ok())
            .map(|v| v / 10.);
        // 休眠时 ACTMON 可能保留上一次活动的平均值，不作为当前忙碌率展示
        let show_load = runtime != Some("suspended");
        let mut content = card(
            SharedString::from(format!("monitor-domain-{id}")),
            CatalogIcon::Layers,
            title,
            Some(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(engine.name)
                    .into_any_element(),
            ),
            cx,
        )
        .child(compact_detail_row(
            SharedString::from(format!("{id}-runtime")).into(),
            t!("monitor.jetson.runtime"),
            runtime.map(runtime_label).unwrap_or_else(|| "—".into()),
            cx,
        ))
        .child(meter(
            format!("{id}-load"),
            t!("monitor.jetson.load"),
            show_load.then_some(load).flatten(),
            cx,
        ));
        if let Some(metric) = usage
            && (metric.status != MetricStatus::Ready || metric.stale)
        {
            content = content.child(status_row(
                &format!("{id}-usage"),
                &metric.status,
                metric.stale.then_some(metric.observed).flatten(),
                cx,
            ));
        }
        if runtime == Some("suspended") {
            content = content.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("monitor.jetson.sleep_note")),
            );
        }
        if let Some(metric) = state.sources.get("device/power/runtime_status")
            && (metric.status != MetricStatus::Ready || metric.stale)
        {
            content = content.child(status_row(
                &format!("{id}-runtime"),
                &metric.status,
                metric.stale.then_some(metric.observed).flatten(),
                cx,
            ));
        }
        let frequency = state
            .current("cur_freq")
            .and_then(|s| s.parse::<u64>().ok())
            .map(|v| format!("{:.0} MHz", v as f64 / 1_000_000.))
            .unwrap_or_else(|| "—".into());
        content = content
            .child(compact_detail_row(
                SharedString::from(format!("{id}-frequency")).into(),
                t!("monitor.hardware.frequency"),
                frequency.into(),
                cx,
            ))
            .child(compact_detail_row(
                SharedString::from(format!("{id}-awake")).into(),
                t!("monitor.jetson.awake_share"),
                state
                    .awake
                    .value
                    .filter(|_| state.awake.status == MetricStatus::Ready && !state.awake.stale)
                    .map(|v| format!("{v:.1}%"))
                    .unwrap_or_else(|| "—".into())
                    .into(),
                cx,
            ));
        if usage.is_some_and(|m| {
            matches!(
                m.status,
                MetricStatus::PermissionDenied
                    | MetricStatus::SudoDenied
                    | MetricStatus::SudoUnavailable
            )
        }) {
            let dispatch = dispatch.clone();
            content = content.child(
                Button::new(SharedString::from(format!("{id}-authorize")))
                    .ghost()
                    .small()
                    .w_full()
                    .label(t!("monitor.access.configure"))
                    .on_click(move |_, window, cx| {
                        dispatch.dispatch_action(&ConfigureMonitorAccess, window, cx)
                    }),
            );
        }
        let dispatch = dispatch.clone();
        content = content.child(
            Button::new(SharedString::from(format!("monitor-details-{id}")))
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
                    dispatch.dispatch_action(&ToggleMonitorMetric(key.clone()), window, cx)
                }),
        );
        if expanded {
            let mut rows = v_flex().min_w_0().gap_3().child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("monitor.jetson.note")),
            );
            for field in FIELDS.iter().copied().chain(["usage"]) {
                let Some(metric) = state.sources.get(field) else {
                    continue;
                };
                let row_id = format!("{id}-detail-{field}");
                let value = metric
                    .value
                    .as_deref()
                    .map(|v| {
                        if field.ends_with("_time") {
                            format!("{v} ms")
                        } else if field == "cur_freq" {
                            format!("{v} Hz")
                        } else if field == "usage" {
                            format!("{v} / 1000")
                        } else {
                            v.to_owned()
                        }
                    })
                    .unwrap_or_else(|| status_text(&metric.status).to_string());
                rows = rows
                    .child(compact_detail_row(
                        SharedString::from(row_id.clone()).into(),
                        field_label(field),
                        value.into(),
                        cx,
                    ))
                    .child(status_row(
                        &row_id,
                        &metric.status,
                        metric.stale.then_some(metric.observed).flatten(),
                        cx,
                    ))
                    .child(stacked_detail_row(
                        SharedString::from(format!("{row_id}-source")).into(),
                        t!("monitor.hardware.source"),
                        source(engine, field).into(),
                        cx,
                    ));
                if let Some(at) = metric.observed {
                    rows = rows.child(
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
            content = content.child(rows);
        }
        cards.push(content.into_any_element());
    }
    cards
}
