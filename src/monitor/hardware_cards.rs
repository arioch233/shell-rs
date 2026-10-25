//! 硬件读数沿用系统监控的卡片和进度条，诊断信息按卡片展开

use std::collections::{BTreeMap, HashSet};

use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::controller::HostMonitorSnapshot;
use super::monitor_panel::{status_row, status_text};
use super::snapshot::{CollectorDomain, HardwareMetric, Metric, MetricStatus, Unit};
use crate::app::{CatalogIcon, SetMonitorSettings, ToggleMonitorMetric};
use crate::i18n::{t, tn};
use crate::shared::{card, meter, numeric, stacked_detail_row};

pub(super) fn render(
    host: &HostMonitorSnapshot,
    details: &HashSet<String>,
    dispatch: &FocusHandle,
    cx: &App,
) -> Vec<AnyElement> {
    let mut cards = super::tegra_cards::render(&host.tegra, details, dispatch, cx);
    if let Some(view) = super::dvpp_cards::render(&host.dvpp, details, dispatch, cx) {
        cards.push(view);
    }
    for domain in [
        CollectorDomain::Devfreq,
        CollectorDomain::Rga,
        CollectorDomain::Npu,
        CollectorDomain::Mpp,
        CollectorDomain::Thermal,
        CollectorDomain::Psi,
    ] {
        if !host.capabilities.supports(domain) {
            continue;
        }
        let Some(metric) = host.hardware.get(&domain) else {
            continue;
        };
        let rows = metric.value.as_deref().unwrap_or_default();
        if rows.is_empty() && matches!(metric.status, MetricStatus::Ready | MetricStatus::Loading) {
            continue;
        }
        // devfreq 的每个设备有独立卡片，GPU 和 DDR 不共享负载或频率
        let mut groups: BTreeMap<&str, Vec<&HardwareMetric>> = BTreeMap::new();
        for row in rows {
            let key = if domain == CollectorDomain::Devfreq {
                row.source
                    .strip_prefix("/sys/class/devfreq/")
                    .and_then(|path| path.split('/').next())
                    .unwrap_or(domain.key())
            } else {
                domain.key()
            };
            groups.entry(key).or_default().push(row);
        }
        if groups.is_empty() {
            groups.insert(domain.key(), Vec::new());
        }
        let device_names: Vec<_> = groups.keys().copied().collect();
        for (device, rows) in &groups {
            let device = *device;
            if host.tegra.engines.contains_key(device) {
                continue;
            }
            let rows = rows.clone();
            // Jetson 的板载 GPU 已由加速器卡片合并利用率、温度和频率
            if host.capabilities.tegra
                && device.ends_with(".gpu")
                && host
                    .accelerators
                    .value
                    .as_ref()
                    .is_some_and(|reading| reading.devices.iter().any(|device| device.integrated))
            {
                continue;
            }
            let engine = host
                .capabilities
                .tegra
                .then(|| tegra_engine(device))
                .flatten();
            let key = format!("hardware/{}/{device}", domain.key());
            let expanded = details.contains(&key);
            let count = unit_count(&rows, domain);
            let subtitle = if let Some((_, abbreviation)) = &engine {
                Some(t!("monitor.hardware.clock_only", engine = abbreviation).to_string())
            } else if domain == CollectorDomain::Devfreq && device.contains("gpu") {
                Some(
                    gpu_name(&rows)
                        .unwrap_or_else(|| t!("monitor.hardware.unknown_model").to_string()),
                )
            } else {
                card_subtitle(&rows, domain, metric.stale)
            };
            let (title, icon) = match domain {
                CollectorDomain::Rga => ("RGA".into(), CatalogIcon::ScanLine),
                CollectorDomain::Npu => ("NPU".into(), CatalogIcon::Cpu),
                CollectorDomain::Mpp => ("MPP / VPU".into(), CatalogIcon::Layers),
                CollectorDomain::Thermal => (t!("monitor.thermal"), CatalogIcon::Thermometer),
                CollectorDomain::Psi => (t!("monitor.pressure"), CatalogIcon::Activity),
                _ if engine.is_some() => {
                    let (name, _) = engine.as_ref().unwrap();
                    let family = device.rsplit('.').next().unwrap_or(device);
                    let siblings: Vec<_> = device_names
                        .iter()
                        .filter(|name| name.rsplit('.').next() == Some(family))
                        .collect();
                    let name = if siblings.len() > 1 {
                        let ix = siblings
                            .iter()
                            .position(|name| **name == device)
                            .unwrap_or(0);
                        format!("{name} · {ix}").into()
                    } else {
                        name.clone()
                    };
                    (name, CatalogIcon::Layers)
                }
                _ if device.contains("gpu") => ("GPU".into(), CatalogIcon::Gpu),
                _ if device.contains("dmc") || device.contains("ddr") => {
                    ("DDR".into(), CatalogIcon::MemoryStick)
                }
                _ => (device.to_owned().into(), CatalogIcon::Activity),
            };
            let meta = if metric.stale {
                Some(t!("monitor.state.stale"))
            } else if count > 0 && domain == CollectorDomain::Rga {
                Some(tn!("monitor.hardware.units", count))
            } else if count > 0 && domain == CollectorDomain::Npu {
                Some(tn!("monitor.cpu.cores", count))
            } else if domain == CollectorDomain::Thermal && count > 0 {
                Some(tn!("monitor.hardware.sensors", count))
            } else {
                None
            };
            let toggle = dispatch.clone();
            let settings = dispatch.clone();
            let permission_denied = rows.iter().any(|row| {
                matches!(
                    row.status,
                    MetricStatus::PermissionDenied
                        | MetricStatus::SudoDenied
                        | MetricStatus::SudoUnavailable
                )
            }) || matches!(
                metric.status,
                MetricStatus::PermissionDenied
                    | MetricStatus::SudoDenied
                    | MetricStatus::SudoUnavailable
            );
            let primary: Vec<_> = rows
                .iter()
                .copied()
                .filter(|row| primary_metric(row, &rows, domain, metric.stale))
                .collect();
            let view = card(
                SharedString::from(format!("monitor-domain-{}-{device}", domain.key())),
                icon,
                title,
                meta.map(|meta| {
                    div()
                        .id(SharedString::from(format!(
                            "monitor-device-{}-{device}",
                            domain.key()
                        )))
                        .test_support()
                        .aria_label(meta.clone())
                        .min_w_0()
                        .max_w(rems(9.))
                        .truncate()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(meta.clone())
                        .tooltip(move |window, cx| Tooltip::new(meta.clone()).build(window, cx))
                        .into_any_element()
                }),
                cx,
            )
            .when_some(subtitle, |view, subtitle| {
                view.child(
                    div()
                        .id(SharedString::from(format!(
                            "monitor-subtitle-{}-{device}",
                            domain.key()
                        )))
                        .test_support()
                        .aria_label(subtitle.clone())
                        .min_w_0()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(subtitle),
                )
            })
            .when(
                metric.stale || metric.status != MetricStatus::Ready,
                |view| {
                    view.child(status_row(
                        domain.key(),
                        &metric.status,
                        metric.stale.then_some(metric.observed).flatten(),
                        cx,
                    ))
                },
            )
            .when(
                domain == CollectorDomain::Thermal && !primary.is_empty(),
                |view| view.child(render_temperatures(&primary, metric.stale, cx)),
            )
            .when(
                domain != CollectorDomain::Thermal && !primary.is_empty(),
                |view| {
                    view.child(
                        v_flex().min_w_0().gap_3().children(
                            primary
                                .iter()
                                .map(|row| render_metric(row, metric.stale, domain, cx)),
                        ),
                    )
                },
            )
            .when(domain == CollectorDomain::Mpp, |view| {
                view.child(super::mpp_cards::render(
                    &host.mpp,
                    false,
                    details.contains("mpp/sessions/all"),
                    dispatch,
                    cx,
                ))
            })
            .when(
                domain != CollectorDomain::Mpp
                    && primary.is_empty()
                    && metric.status == MetricStatus::Ready,
                |view| {
                    view.child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(if domain == CollectorDomain::Npu {
                                t!("monitor.hardware.no_core_load")
                            } else {
                                t!("monitor.hardware.no_load")
                            }),
                    )
                },
            )
            .when(permission_denied, |view| {
                view.child(
                    Button::new(SharedString::from(format!(
                        "monitor-permission-{}-{device}",
                        domain.key()
                    )))
                    .outline()
                    .small()
                    .label(t!("monitor.sudo.configure"))
                    .on_click(move |_, window, cx| {
                        settings.dispatch_action(&SetMonitorSettings(true), window, cx)
                    }),
                )
            })
            .child(
                Button::new(SharedString::from(format!(
                    "monitor-details-{}-{device}",
                    domain.key()
                )))
                .ghost()
                .small()
                .w_full()
                .justify_between()
                .label(t!("monitor.hardware.details"))
                .text_color(cx.theme().muted_foreground)
                .child(
                    Icon::new(if expanded {
                        CatalogIcon::ChevronUp
                    } else {
                        CatalogIcon::ChevronDown
                    })
                    .xsmall(),
                )
                .toggled(expanded)
                .on_click(move |_, window, cx| {
                    toggle.dispatch_action(&ToggleMonitorMetric(key.clone()), window, cx)
                }),
            )
            .when(expanded, |view| {
                if domain == CollectorDomain::Mpp {
                    let load_rows: Vec<_> = rows
                        .iter()
                        .copied()
                        .filter(|row| row.source == "/proc/mpp_service/load")
                        .collect();
                    view.child(super::mpp_cards::render(
                        &host.mpp,
                        true,
                        details.contains("mpp/sessions/all"),
                        dispatch,
                        cx,
                    ))
                    .child(render_details(&load_rows, metric, domain, device, cx))
                } else {
                    view.child(render_details(&rows, metric, domain, device, cx))
                }
            });
            cards.push(view.into_any_element());
        }
    }
    cards
}

fn unit_count(rows: &[&HardwareMetric], domain: CollectorDomain) -> usize {
    rows.iter()
        .filter(|row| match domain {
            CollectorDomain::Npu => row.id.starts_with("rockchip/npu/core/"),
            CollectorDomain::Rga => row.id.starts_with("rockchip/rga/scheduler/"),
            CollectorDomain::Thermal => row.unit == Unit::Celsius,
            _ => false,
        })
        .count()
}

fn subtitle_frequency<'a>(rows: &[&'a HardwareMetric], stale: bool) -> Option<&'a HardwareMetric> {
    if stale {
        return None;
    }
    let mut frequencies = rows.iter().copied().filter(|row| {
        row.unit == Unit::Hertz && row.status == MetricStatus::Ready && row.value.is_some()
    });
    let first = frequencies.next()?;
    frequencies.next().is_none().then_some(first)
}

fn primary_metric(
    row: &HardwareMetric,
    rows: &[&HardwareMetric],
    domain: CollectorDomain,
    stale: bool,
) -> bool {
    if domain == CollectorDomain::Mpp && row.source != "/proc/mpp_service/load" {
        return false;
    }
    if row.source.starts_with("/sys/class/devfreq/")
        && (row.source.ends_with("/name") || row.source.ends_with("/gpuinfo"))
    {
        return false;
    }
    if domain == CollectorDomain::Npu
        && (row.source.ends_with("/version")
            || row.source.ends_with("/driver_version")
            || row.source.ends_with("/name"))
    {
        return false;
    }
    if row.unit == Unit::Text {
        return row.status != MetricStatus::Ready;
    }
    if domain == CollectorDomain::Npu {
        if row.source.starts_with("/sys/class/devfreq/") && row.source.ends_with("/load") {
            return false;
        }
        if subtitle_frequency(rows, stale).is_some_and(|frequency| frequency.id == row.id) {
            return false;
        }
    }
    true
}

fn card_subtitle(rows: &[&HardwareMetric], domain: CollectorDomain, stale: bool) -> Option<String> {
    let mut parts = Vec::new();
    if domain == CollectorDomain::Npu && !stale {
        if let Some(frequency) = subtitle_frequency(rows, stale).and_then(metric_value) {
            parts.push(frequency);
        }
        if let Some(version) = rows
            .iter()
            .filter(|row| row.status == MetricStatus::Ready)
            .find_map(|row| row.detail.strip_prefix("RKNPU driver:").map(str::trim))
        {
            parts.push(t!("monitor.hardware.driver", version = version).to_string());
        }
    } else if domain == CollectorDomain::Rga {
        let mut kinds = BTreeMap::<String, usize>::new();
        for row in rows
            .iter()
            .filter(|row| row.id.starts_with("rockchip/rga/scheduler/"))
        {
            let kind = row
                .label
                .split(" · ")
                .next()
                .unwrap_or("RGA")
                .to_uppercase();
            *kinds.entry(kind).or_default() += 1;
        }
        parts.extend(
            kinds
                .into_iter()
                .rev()
                .map(|(kind, count)| format!("{count} × {kind}")),
        );
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

// 型号来自驱动读数；地址节点和驱动未知型号不作为硬件名称
fn gpu_name(rows: &[&HardwareMetric]) -> Option<String> {
    for suffix in ["/gpuinfo", "/name"] {
        for row in rows
            .iter()
            .filter(|row| row.source.ends_with(suffix) && row.status == MetricStatus::Ready)
        {
            let text = row.detail.trim();
            if !["Mali-", "Immortalis-", "Adreno", "PowerVR"]
                .iter()
                .any(|prefix| text.starts_with(prefix))
            {
                continue;
            }
            // gpuinfo 同时包含型号、核心数、修订号和产品 ID，后两项只在详情展示
            let name = text
                .split_whitespace()
                .take_while(|word| {
                    !word.starts_with("0x")
                        && !word.strip_prefix('r').is_some_and(|revision| {
                            revision.split_once('p').is_some_and(|(major, minor)| {
                                !major.is_empty()
                                    && !minor.is_empty()
                                    && major
                                        .bytes()
                                        .chain(minor.bytes())
                                        .all(|byte| byte.is_ascii_digit())
                            })
                        })
                })
                .collect::<Vec<_>>()
                .join(" ");
            return Some(name);
        }
    }
    None
}

/// Jetson devfreq 节点按功能命名，地址和原始名称仅放入采集详情
pub(super) fn tegra_engine(device: &str) -> Option<(SharedString, &'static str)> {
    Some(match device.rsplit('.').next()? {
        "nvenc" => (t!("monitor.jetson.encoder"), "NVENC"),
        "nvdec" => (t!("monitor.jetson.decoder"), "NVDEC"),
        "nvjpg" => (t!("monitor.jetson.jpeg"), "NVJPG"),
        "vic" => (t!("monitor.jetson.compositor"), "VIC"),
        "ofa" => (t!("monitor.jetson.optical_flow"), "OFA"),
        _ => return None,
    })
}

fn thermal_group(name: &str) -> usize {
    match name {
        "soc-thermal" | "center-thermal" | "tj-thermal" => 0,
        _ if name.starts_with("soc") && thermal_label(name) != name => 0,
        "littlecore-thermal" | "cpu-thermal" => 1,
        "gpu-thermal" | "npu-thermal" => 2,
        _ if name.starts_with("cv") && thermal_label(name) != name => 2,
        _ if name.starts_with("bigcore") && thermal_label(name) != name => 1,
        _ => 3,
    }
}

fn render_temperatures(rows: &[&HardwareMetric], stale: bool, cx: &App) -> impl IntoElement {
    let highest = (!stale)
        .then(|| {
            rows.iter()
                .copied()
                .filter(|row| {
                    row.unit == Unit::Celsius
                        && row.status == MetricStatus::Ready
                        && row.value.is_some()
                })
                .max_by(|left, right| left.value.unwrap().total_cmp(&right.value.unwrap()))
        })
        .flatten();
    v_flex()
        .min_w_0()
        .gap_3()
        .when_some(highest, |view, row| {
            let value = metric_value(row).unwrap();
            let name = thermal_label(&row.label);
            view.child(
                h_flex()
                    .id("monitor-thermal-highest")
                    .test_support()
                    .aria_label(format!(
                        "{}: {value} · {name}",
                        t!("monitor.thermal.highest")
                    ))
                    .min_w_0()
                    .gap_3()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(t!("monitor.thermal.highest")),
                            )
                            .child(div().text_sm().child(name)),
                    )
                    .child(
                        numeric(value, cx)
                            .text_lg()
                            .font_weight(FontWeight::MEDIUM)
                            .flex_shrink_0(),
                    ),
            )
        })
        .children(
            [
                (0, t!("monitor.thermal.group.soc")),
                (1, "CPU".into()),
                (2, t!("monitor.thermal.group.accelerators")),
                (3, t!("monitor.thermal.group.additional")),
            ]
            .into_iter()
            .filter_map(|(group, title)| {
                let members: Vec<_> = rows
                    .iter()
                    .filter(|row| thermal_group(&row.label) == group)
                    .collect();
                if members.is_empty() {
                    return None;
                }
                Some(
                    v_flex()
                        .id(SharedString::from(format!("monitor-thermal-group-{group}")))
                        .test_support()
                        .aria_label(title.clone())
                        .min_w_0()
                        .gap_2()
                        .pt_2()
                        .border_t_1()
                        .border_color(cx.theme().border)
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(cx.theme().muted_foreground)
                                .child(title),
                        )
                        .children(
                            members
                                .into_iter()
                                .map(|row| render_metric(row, stale, CollectorDomain::Thermal, cx)),
                        ),
                )
            }),
        )
}

fn thermal_label(name: &str) -> SharedString {
    match name {
        "cpu-thermal" => "CPU".into(),
        "tj-thermal" => t!("monitor.thermal.junction"),
        "soc-thermal" => t!("monitor.thermal.soc"),
        "littlecore-thermal" => t!("monitor.thermal.little"),
        "center-thermal" => t!("monitor.thermal.center"),
        "gpu-thermal" => "GPU".into(),
        "npu-thermal" => "NPU".into(),
        _ => {
            for (prefix, label) in [
                ("soc", "monitor.thermal.soc_zone"),
                ("cv", "monitor.thermal.cv_zone"),
            ] {
                if let Some(index) = name
                    .strip_prefix(prefix)
                    .and_then(|name| name.strip_suffix("-thermal"))
                    && !index.is_empty()
                    && index.bytes().all(|byte| byte.is_ascii_digit())
                {
                    return t!(label, index = index);
                }
            }
            if let Some(index) = name
                .strip_prefix("bigcore")
                .and_then(|name| name.strip_suffix("-thermal"))
                && !index.is_empty()
                && index.bytes().all(|byte| byte.is_ascii_digit())
            {
                return t!("monitor.thermal.big", index = index);
            }
            name.to_owned().into()
        }
    }
}

fn metric_value(row: &HardwareMetric) -> Option<String> {
    row.value.map(|value| match row.unit {
        Unit::Percent => format!("{value:.1}%"),
        Unit::Hertz => format!("{:.0} MHz", value / 1_000_000.0),
        Unit::Celsius => format!("{value:.1} °C"),
        Unit::Text => String::new(),
    })
}

fn render_metric(
    row: &HardwareMetric,
    stale: bool,
    domain: CollectorDomain,
    cx: &App,
) -> impl IntoElement {
    let value = metric_value(row);
    let status = (stale || row.status != MetricStatus::Ready).then(|| {
        if row.value.is_some() {
            if row.status == MetricStatus::Ready {
                t!("monitor.state.stale")
            } else {
                t!(
                    "monitor.hardware.previous",
                    status = status_text(&row.status)
                )
            }
        } else {
            status_text(&row.status)
        }
    });
    let label = metric_label(row, domain);
    let display = match (&value, &status) {
        (Some(value), Some(status)) => format!("{value} · {status}"),
        (Some(value), None) => value.clone(),
        (None, Some(status)) => status.to_string(),
        _ => row.detail.clone(),
    };
    v_flex()
        .id(SharedString::from(format!("monitor-metric-{}", row.id)))
        .test_support()
        .aria_label(format!("{label}: {display}"))
        .min_w_0()
        .gap_1p5()
        .when(row.unit == Unit::Percent, |view| {
            view.child(meter(
                format!("monitor-value-{}", row.id),
                label.clone(),
                row.value.map(|value| value as f32),
                cx,
            ))
        })
        .when(row.unit != Unit::Percent, |view| {
            view.child(
                h_flex()
                    .min_w_0()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(label),
                    )
                    .when_some(value, |view, value| {
                        view.child(
                            numeric(value.clone(), cx)
                                .id(SharedString::from(format!("monitor-value-{}", row.id)))
                                .test_support()
                                .aria_label(value)
                                .text_sm()
                                .flex_shrink_0(),
                        )
                    }),
            )
        })
        .when_some(status, |view, status| {
            view.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(status),
            )
        })
}

fn metric_label(row: &HardwareMetric, domain: CollectorDomain) -> SharedString {
    if domain == CollectorDomain::Npu
        && let Some(core) = row.id.strip_prefix("rockchip/npu/core/")
    {
        return t!("monitor.hardware.core", index = core);
    }
    if row.source.starts_with("/sys/class/devfreq/") {
        return if row.source.ends_with("/cur_freq") {
            t!("monitor.hardware.frequency")
        } else if row.source.ends_with("/load") {
            if domain == CollectorDomain::Npu {
                t!("monitor.hardware.devfreq_load")
            } else {
                t!("monitor.hardware.usage")
            }
        } else {
            row.label.clone().into()
        };
    }
    if domain == CollectorDomain::Rga
        && let Some(index) = row.id.strip_prefix("rockchip/rga/scheduler/")
    {
        let kind = row
            .label
            .split(" · ")
            .next()
            .unwrap_or("RGA")
            .to_uppercase();
        return t!("monitor.hardware.rga_core", kind = kind, index = index);
    }
    if domain == CollectorDomain::Thermal {
        return thermal_label(&row.label);
    }
    if row.source.ends_with("/load") && row.unit == Unit::Text {
        return t!("monitor.hardware.load");
    }
    row.label.clone().into()
}

fn detail_heading(row: &HardwareMetric, domain: CollectorDomain) -> SharedString {
    if row.source.ends_with("/gpuinfo") {
        return t!("monitor.hardware.model");
    }
    if row.source.ends_with("/name") {
        return t!("monitor.hardware.device_name");
    }
    if row.source.ends_with("/version") || row.source.ends_with("/driver_version") {
        return t!("monitor.hardware.driver_info");
    }
    if row.source.ends_with("/load") && !row.source.contains("/devfreq/") {
        return t!("monitor.hardware.load");
    }
    metric_label(row, domain)
}

fn render_details(
    rows: &[&HardwareMetric],
    metric: &Metric<Vec<HardwareMetric>>,
    domain: CollectorDomain,
    device: &str,
    cx: &App,
) -> impl IntoElement {
    let mut sources = HashSet::new();
    v_flex()
        .id(SharedString::from(format!(
            "monitor-sources-{}-{device}",
            domain.key()
        )))
        .test_support()
        .min_w_0()
        .gap_3()
        .pt_3()
        .border_t_1()
        .border_color(cx.theme().border)
        .text_xs()
        .child(
            h_flex()
                .gap_2()
                .flex_wrap()
                .child(
                    div()
                        .text_color(cx.theme().muted_foreground)
                        .child(t!("monitor.hardware.sample_status")),
                )
                .child(if metric.stale {
                    t!("monitor.state.stale")
                } else {
                    status_text(&metric.status)
                }),
        )
        .when_some(metric.observed, |view, at| {
            view.child(div().text_color(cx.theme().muted_foreground).child(t!(
                "monitor.hardware.observed",
                seconds = cx.background_executor().now().saturating_duration_since(at).as_secs()
            )))
        })
        .when(domain == CollectorDomain::Devfreq, |view| {
            view.child(stacked_detail_row(
                SharedString::from(format!("monitor-node-{}-{device}", domain.key())).into(),
                t!("monitor.hardware.device_node"),
                device.to_owned().into(),
                cx,
            ))
        })
        .when(
            matches!(domain, CollectorDomain::Rga | CollectorDomain::Npu),
            |view| {
                view.child(
                    div()
                        .text_color(cx.theme().muted_foreground)
                        .child(t!("monitor.hardware.independent_load")),
                )
            },
        )
        .when(
            domain == CollectorDomain::Npu
                && rows
                    .iter()
                    .any(|row| row.source.starts_with("/sys/class/devfreq/")),
            |view| {
                view.child(
                    v_flex()
                        .min_w_0()
                        .gap_2()
                        .children(
                            rows.iter()
                                .filter(|row| {
                                    row.source.starts_with("/sys/class/devfreq/")
                                        && row.unit != Unit::Text
                                })
                                .map(|row| {
                                    let label = metric_label(row, domain);
                                    let value = metric_value(row)
                                        .unwrap_or_else(|| status_text(&row.status).to_string());
                                    h_flex()
                                        .gap_2()
                                        .min_w_0()
                                        .id(SharedString::from(format!(
                                            "monitor-diagnostic-{}",
                                            row.id
                                        )))
                                        .test_support()
                                        .aria_label(format!("{label}: {value}"))
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(label),
                                        )
                                        .child(numeric(value, cx).flex_shrink_0())
                                }),
                        )
                        .child(
                            div()
                                .text_color(cx.theme().muted_foreground)
                                .child(t!("monitor.hardware.devfreq_explanation")),
                        ),
                )
            },
        )
        .children(rows.iter().filter_map(|row| {
            if !sources.insert(&row.source) {
                return None;
            }
            Some(
                v_flex()
                    .min_w_0()
                    .gap_2()
                    .pt_2()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .child(detail_heading(row, domain)),
                    )
                    .when(domain == CollectorDomain::Rga, |view| {
                        let units: Vec<_> = rows
                            .iter()
                            .filter(|unit| unit.source == row.source)
                            .filter_map(|unit| {
                                let index = unit.id.strip_prefix("rockchip/rga/scheduler/")?;
                                let kind = unit
                                    .label
                                    .split(" · ")
                                    .next()
                                    .unwrap_or("RGA")
                                    .to_uppercase();
                                Some(stacked_detail_row(
                                    SharedString::from(format!("monitor-rga-unit-{}", unit.id))
                                        .into(),
                                    t!("monitor.hardware.core", index = index),
                                    t!("monitor.hardware.rga_identity", kind = kind, index = index),
                                    cx,
                                ))
                            })
                            .collect();
                        view.when(!units.is_empty(), |view| {
                            view.children(units).child(
                                div()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(t!("monitor.hardware.rga_scheduler_explanation")),
                            )
                        })
                    })
                    .when(domain == CollectorDomain::Thermal, |view| {
                        view.child(stacked_detail_row(
                            SharedString::from(format!("monitor-original-name-{}", row.id)).into(),
                            t!("monitor.hardware.original_name"),
                            row.label.clone().into(),
                            cx,
                        ))
                    })
                    .when(row.status != MetricStatus::Ready, |view| {
                        view.child(stacked_detail_row(
                            SharedString::from(format!("monitor-source-status-{}", row.id)).into(),
                            t!("monitor.hardware.sample_status"),
                            status_text(&row.status),
                            cx,
                        ))
                    })
                    .when(!row.detail.is_empty(), |view| {
                        view.child(stacked_detail_row(
                            SharedString::from(format!("monitor-detail-value-{}", row.id)).into(),
                            t!("monitor.hardware.raw_output"),
                            row.detail.clone().into(),
                            cx,
                        ))
                    })
                    .child(stacked_detail_row(
                        SharedString::from(format!("monitor-source-{}", row.id)).into(),
                        t!("monitor.hardware.source"),
                        row.source.clone().into(),
                        cx,
                    )),
            )
        }))
}

#[cfg(test)]
mod tests {
    use super::{gpu_name, primary_metric};
    use crate::monitor::snapshot::{CollectorDomain, HardwareMetric, MetricStatus};

    #[test]
    fn gpu_identity_uses_driver_model_and_keeps_revisions_out_of_the_title() {
        for (reading, expected) in [
            ("Mali-G610 MP4 r0p0 0xA8670000", "Mali-G610 MP4"),
            ("Mali-G610 4 cores r1p0 0xA8670000", "Mali-G610 4 cores"),
        ] {
            let row = HardwareMetric::state(
                "/sys/class/devfreq/fb000000.gpu/device/gpuinfo",
                MetricStatus::Ready,
                reading,
            );
            assert_eq!(gpu_name(&[&row]).as_deref(), Some(expected));
            assert!(!primary_metric(
                &row,
                &[&row],
                CollectorDomain::Devfreq,
                false
            ));
        }
        for (status, value) in [
            (MetricStatus::Ready, "fb000000.gpu"),
            (MetricStatus::Ready, "(Unknown Mali GPU) 4 cores r0p0 0x0"),
            (MetricStatus::PermissionDenied, "Mali-G610"),
        ] {
            let row = HardwareMetric::state("/sys/class/devfreq/fb000000.gpu/name", status, value);
            assert_eq!(gpu_name(&[&row]), None);
            assert!(!primary_metric(
                &row,
                &[&row],
                CollectorDomain::Devfreq,
                false
            ));
        }
    }
}
