use std::cell::Cell;
use std::collections::HashSet;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    group_box::GroupBoxVariants as _,
    h_flex,
    progress::Progress,
    scroll::ScrollableElement as _,
    switch::Switch,
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::controller::MonitorController;
use super::model::{
    DiskUsage, InterfaceRate, Memory, Snapshot, SystemInfo, bars_per_row, format_rate,
};
use super::snapshot::MetricStatus;
use super::{CollectorDomain, MonitorDetail, MonitorPage};
use crate::app::{
    CatalogIcon, RefreshMonitor, SelectMonitorPage, SelectMonitorProcesses, SetMonitorSettings,
    SetMonitorSudo, ToggleMonitorDetail,
};
use crate::gpu::GpuCards;
use crate::i18n::{t, tn};
use crate::processes::{Process, ProcessDetails, ProcessPanel, ProcessSort};
use crate::shared::{
    Segment, SegmentedControl, amount, card, format_bytes, format_duration, format_percent, gauge,
    load_color, meter, numeric,
};
use crate::terminal::{ExecTarget, RemoteTerminalId};

/// 统一监控的展示状态，采样及缓存由控制器持有
pub struct MonitorPanel {
    controller: Entity<MonitorController>,
    preference_key: Option<String>,
    preference_generation: Option<(RemoteTerminalId, u64)>,
    preferences_by_host: std::collections::HashMap<String, super::MonitorPreferences>,
    processes: Entity<ProcessPanel>,
    page: MonitorPage,
    gpu_processes: bool,
    allow_sudo: bool,
    settings_open: bool,
    cards: GpuCards,
    cores_unfolded: bool,
    cores_width: Rc<Cell<Option<Pixels>>>,
    interfaces_unfolded: bool,
    details: HashSet<String>,
    dispatch: FocusHandle,
    focus_handle: FocusHandle,
    _subscription: Subscription,
}

impl MonitorPanel {
    pub fn new(dispatch: FocusHandle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let controller = cx.new(MonitorController::new);
        let processes = cx.new(|cx| ProcessPanel::new(dispatch.clone(), window, cx));
        let focus_handle = cx.focus_handle();
        let subscription = cx.observe(&controller, |this, _, cx| {
            this.sync_processes(cx);
            cx.notify();
        });
        Self {
            controller,
            preference_key: None,
            preference_generation: None,
            preferences_by_host: std::collections::HashMap::new(),
            processes,
            page: MonitorPage::Overview,
            gpu_processes: false,
            allow_sudo: false,
            settings_open: false,
            cards: GpuCards::new(dispatch.clone(), focus_handle.clone()),
            cores_unfolded: false,
            cores_width: Rc::new(Cell::new(None)),
            interfaces_unfolded: false,
            details: HashSet::new(),
            dispatch,
            focus_handle,
            _subscription: subscription,
        }
    }
    pub fn preferences(&self) -> super::MonitorPreferences {
        let (gpu_all, gpu_toggled) = self.cards.preferences();
        super::MonitorPreferences {
            allow_sudo: self.allow_sudo,
            page: self.page,
            gpu_processes: self.gpu_processes,
            cores: self.cores_unfolded,
            interfaces: self.interfaces_unfolded,
            details: self.details.clone(),
            gpu_all,
            gpu_toggled,
        }
    }
    pub fn persistent_preferences(&self) -> super::MonitorPreferences {
        let mut preferences = self.preferences();
        preferences
            .gpu_toggled
            .retain(|key| !key.starts_with("GPU ") && !key.is_empty());
        preferences.details.retain(|key| {
            key.starts_with("hardware/")
                || key.starts_with("rockchip/")
                || key.starts_with("/sys/class/devfreq/")
                || key.starts_with("/proc/pressure/")
        });
        preferences
    }
    pub fn restore_preferences(
        &mut self,
        key: String,
        preferences: super::MonitorPreferences,
        cx: &mut Context<Self>,
    ) {
        if self.preference_key.as_ref() == Some(&key) {
            return;
        }
        if let Some(previous) = self.preference_key.take() {
            self.preferences_by_host
                .insert(previous, self.preferences());
        }
        let preferences = self
            .preferences_by_host
            .get(&key)
            .cloned()
            .unwrap_or(preferences);
        self.preference_key = Some(key);
        self.settings_open = false;
        self.allow_sudo = preferences.allow_sudo;
        self.gpu_processes = preferences.gpu_processes;
        self.cores_unfolded = preferences.cores;
        self.interfaces_unfolded = preferences.interfaces;
        self.details = preferences.details;
        self.cards
            .restore(preferences.gpu_all, preferences.gpu_toggled);
        self.page = preferences.page;
        cx.notify();
    }
    fn sync_processes(&mut self, cx: &mut Context<Self>) {
        let controller = self.controller.read(cx);
        let snapshot = controller.snapshot();
        let generation = controller
            .target()
            .and_then(|target| snapshot.map(|host| (target.terminal, host.generation)));
        if generation.is_some()
            && self.preference_generation.is_some()
            && generation != self.preference_generation
        {
            let stable = self.persistent_preferences();
            self.cards.restore(stable.gpu_all, stable.gpu_toggled);
            self.details = stable.details;
        }
        if generation.is_some() {
            self.preference_generation = generation;
        }
        let reading = snapshot.and_then(|host| host.processes.value.clone());
        let problem = snapshot.and_then(|host| {
            if host.processes.stale {
                Some(t!("monitor.state.stale").to_string())
            } else if !matches!(
                host.processes.status,
                MetricStatus::Ready | MetricStatus::Loading
            ) {
                Some(status_text(&host.processes.status).to_string())
            } else {
                None
            }
        });
        let target = controller.target();
        self.processes.update(cx, |panel, cx| {
            panel.set_observation(target, reading, problem, cx)
        });
    }
    pub fn set_target(&mut self, target: Option<ExecTarget>, active: bool, cx: &mut Context<Self>) {
        self.controller.update(cx, |controller, cx| {
            controller.set_processes_active(
                self.page == MonitorPage::Processes && !self.gpu_processes,
                cx,
            );
            controller.set_target(target, active, self.allow_sudo, cx);
        });
        self.sync_processes(cx);
        cx.notify();
    }
    pub fn retain(&mut self, terminals: &[RemoteTerminalId], cx: &mut Context<Self>) {
        self.controller
            .update(cx, |controller, _| controller.retain(terminals));
    }
    pub fn page(&self) -> MonitorPage {
        self.page
    }
    pub fn select_page(&mut self, page: MonitorPage, cx: &mut Context<Self>) {
        self.page = page;
        self.controller.update(cx, |controller, cx| {
            controller
                .set_processes_active(page == MonitorPage::Processes && !self.gpu_processes, cx)
        });
        cx.notify();
    }
    pub fn set_settings_open(&mut self, open: bool, cx: &mut Context<Self>) {
        self.settings_open = open;
        cx.notify();
    }
    pub fn set_allow_sudo(&mut self, allow_sudo: bool, cx: &mut Context<Self>) {
        self.allow_sudo = allow_sudo;
        self.controller.update(cx, |controller, cx| {
            controller.set_allow_sudo(allow_sudo, cx)
        });
        cx.notify();
    }
    pub fn select_processes(&mut self, gpu: bool, cx: &mut Context<Self>) {
        self.gpu_processes = gpu;
        self.controller.update(cx, |controller, cx| {
            controller.set_processes_active(self.page == MonitorPage::Processes && !gpu, cx)
        });
        cx.notify();
    }
    pub fn refresh_target(
        &mut self,
        terminal: RemoteTerminalId,
        generation: u64,
        domain: CollectorDomain,
        cx: &mut Context<Self>,
    ) {
        self.controller.update(cx, |controller, cx| {
            controller.refresh_target(terminal, generation, domain, cx)
        });
    }
    pub fn refresh(&mut self, domain: Option<CollectorDomain>, cx: &mut Context<Self>) {
        self.controller
            .update(cx, |controller, cx| controller.refresh(domain, cx));
    }
    pub fn toggle(&mut self, detail: MonitorDetail, cx: &mut Context<Self>) {
        match detail {
            MonitorDetail::Cores => self.cores_unfolded = !self.cores_unfolded,
            MonitorDetail::Interfaces => self.interfaces_unfolded = !self.interfaces_unfolded,
        }
        cx.notify();
    }
    pub fn toggle_metric(&mut self, id: &str, cx: &mut Context<Self>) {
        if !self.details.remove(id) {
            self.details.insert(id.into());
        }
        cx.notify();
    }
    pub fn toggle_gpu(&mut self, id: &str, cx: &mut Context<Self>) {
        self.cards.toggle(id);
        cx.notify();
    }
    pub fn toggle_all_gpu(&mut self, cx: &mut Context<Self>) {
        if let Some(reading) = self
            .controller
            .read(cx)
            .snapshot()
            .and_then(|host| host.accelerators.value.as_ref())
        {
            self.cards.toggle_all(reading);
        }
        cx.notify();
    }
    pub fn process_label(&self, pid: u32, cx: &App) -> Option<(String, Option<String>)> {
        let controller = self.controller.read(cx);
        let target = controller.target()?;
        let view = target.view.upgrade()?;
        let snapshot = controller.snapshot()?;
        if snapshot.generation != view.read(cx).generation(cx)
            || !matches!(
                view.read(cx).lifecycle(cx),
                crate::terminal::TerminalLifecycle::Running
            )
        {
            return None;
        }
        if self.gpu_processes {
            let host = self.controller.read(cx).snapshot()?;
            if host.accelerators.stale {
                return None;
            }
            let process = host
                .accelerators
                .value
                .as_ref()?
                .processes
                .iter()
                .find(|process| process.pid == pid)?;
            Some((
                process
                    .name
                    .rsplit('/')
                    .next()
                    .unwrap_or(&process.name)
                    .into(),
                process.user.clone(),
            ))
        } else {
            let host = self.controller.read(cx).snapshot()?;
            if host.processes.stale {
                return None;
            }
            let process = self.processes.read(cx).process(pid)?;
            Some((process.name, process.user))
        }
    }
    pub fn process(&self, pid: u32, cx: &App) -> Option<Process> {
        self.processes.read(cx).process(pid)
    }
    pub fn process_details(&self, pid: u32, cx: &App) -> Option<ProcessDetails> {
        self.processes.read(cx).details(pid)
    }
    pub fn sort_processes(&mut self, by: ProcessSort, cx: &mut Context<Self>) {
        self.processes.update(cx, |panel, cx| panel.sort(by, cx));
    }
    pub fn process_domain(&self) -> CollectorDomain {
        if self.gpu_processes {
            CollectorDomain::Accelerator
        } else {
            CollectorDomain::Processes
        }
    }
}

impl Focusable for MonitorPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for MonitorPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let controller = self.controller.read(cx);
        let host = controller.snapshot();
        let select = self.dispatch.clone();
        let refresh = self.dispatch.clone();
        let probe = self.dispatch.clone();
        let sudo = self.dispatch.clone();
        let configure = self.dispatch.clone();
        let page = self.page;
        let settings = self.dispatch.clone();
        let open_settings = !self.settings_open;
        let header = v_flex()
            .flex_shrink_0()
            .px_3()
            .pt_3()
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div().flex_1().min_w_0().child(
                            SegmentedControl::new("monitor-pages")
                                .small()
                                .selected_index(Some(match page {
                                    MonitorPage::Overview => 0,
                                    MonitorPage::Hardware => 1,
                                    MonitorPage::Processes => 2,
                                }))
                                .segments([
                                    Segment::new(t!("monitor.page.overview")),
                                    Segment::new(t!("monitor.page.hardware")),
                                    Segment::new(t!("monitor.page.processes")),
                                ])
                                .on_change(move |ix, window, cx| {
                                    select.dispatch_action(
                                        &SelectMonitorPage(
                                            [
                                                MonitorPage::Overview,
                                                MonitorPage::Hardware,
                                                MonitorPage::Processes,
                                            ][*ix],
                                        ),
                                        window,
                                        cx,
                                    )
                                }),
                        ),
                    )
                    .child(
                        Button::new("monitor-settings-toggle")
                            .ghost()
                            .small()
                            .flex_shrink_0()
                            .icon(CatalogIcon::Settings2)
                            .tooltip(t!("monitor.settings"))
                            .accessibility_label(t!("monitor.settings"))
                            .toggled(self.settings_open)
                            .on_click(move |_, window, cx| {
                                settings.dispatch_action(
                                    &SetMonitorSettings(open_settings),
                                    window,
                                    cx,
                                )
                            }),
                    ),
            )
            .when(self.settings_open, |header| {
                header.child(
                    gpui_kit::component::group_box::GroupBox::new()
                        .fill()
                        .id("monitor-settings")
                        .child(
                            h_flex()
                                .gap_2()
                                .items_start()
                                .child(
                                    Switch::new("monitor-sudo")
                                        .small()
                                        .checked(self.allow_sudo)
                                        .accessibility_label(t!("monitor.sudo.label"))
                                        .tooltip(t!("monitor.sudo.description"))
                                        .on_click(move |enabled, window, cx| {
                                            sudo.dispatch_action(
                                                &SetMonitorSudo(*enabled),
                                                window,
                                                cx,
                                            )
                                        }),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .text_sm()
                                        .child(t!("monitor.sudo.label")),
                                ),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(t!("monitor.sudo.description")),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .flex_wrap()
                                .child(
                                    Button::new("monitor-configure-access")
                                        .outline()
                                        .small()
                                        .label(t!("monitor.access.configure"))
                                        .on_click(move |_, window, cx| {
                                            configure.dispatch_action(
                                                &crate::app::ConfigureMonitorAccess,
                                                window,
                                                cx,
                                            )
                                        }),
                                )
                                .child(
                                    Button::new("monitor-refresh")
                                        .ghost()
                                        .small()
                                        .icon(CatalogIcon::RefreshCw)
                                        .label(t!("tools.refresh"))
                                        .on_click(move |_, window, cx| {
                                            refresh.dispatch_action(
                                                &RefreshMonitor(None),
                                                window,
                                                cx,
                                            )
                                        }),
                                )
                                .child(
                                    Button::new("monitor-probe")
                                        .ghost()
                                        .small()
                                        .icon(IconName::Search)
                                        .label(t!("monitor.reprobe"))
                                        .on_click(move |_, window, cx| {
                                            probe.dispatch_action(
                                                &RefreshMonitor(Some(CollectorDomain::Probe)),
                                                window,
                                                cx,
                                            )
                                        }),
                                ),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(t!("monitor.sampling")),
                        ),
                )
            });
        let cores = CoresLayout {
            unfolded: self.cores_unfolded,
            per_row: self
                .cores_width
                .get()
                .map(|width| bars_per_row(f32::from(width), f32::from(window.rem_size()))),
            width: self.cores_width.clone(),
            panel: cx.entity_id(),
        };
        let mut content = v_flex().gap_3().p_3();
        if let Some(host) = host {
            if page == MonitorPage::Overview {
                content = content.when(
                    host.system.stale || host.system.status != MetricStatus::Ready,
                    |content| {
                        content.child(status_row(
                            "system",
                            &host.system.status,
                            host.system.stale.then_some(host.system.observed).flatten(),
                            cx,
                        ))
                    },
                );
                if let Some(snapshot) = &host.system.value {
                    content = content
                        .when_some(host.info.as_ref(), |panel, info| {
                            panel.child(render_system(info, snapshot.uptime, cx))
                        })
                        .child(render_cpu(
                            host.info.as_ref(),
                            snapshot,
                            &cores,
                            &self.dispatch,
                            cx,
                        ))
                        .when(snapshot.cpu.is_none(), |panel| {
                            panel.child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(t!("monitor.waiting_delta")),
                            )
                        })
                        .child(render_memory(&snapshot.memory, cx))
                        .child(render_network(
                            snapshot,
                            self.interfaces_unfolded,
                            &self.dispatch,
                            cx,
                        ));
                }
                if let Some(disks) = &host.disks.value {
                    content = content.child(render_disks(disks, cx));
                }
                if !matches!(
                    host.disks.status,
                    MetricStatus::Ready | MetricStatus::Loading
                ) {
                    content = content.child(status_row(
                        "disks",
                        &host.disks.status,
                        host.disks.stale.then_some(host.disks.observed).flatten(),
                        cx,
                    ));
                }
            } else if page == MonitorPage::Hardware {
                if let Some(model) = &host.capabilities.board_model {
                    content = content.child(
                        div()
                            .id("monitor-board-model")
                            .test_support()
                            .aria_label(model.clone())
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(model.clone()),
                    );
                }
                if let Some(reading) = &host.accelerators.value {
                    content = content
                        .when(
                            host.accelerators.stale
                                || host.accelerators.status != MetricStatus::Ready,
                            |content| {
                                content.child(status_row(
                                    "accelerator",
                                    &host.accelerators.status,
                                    host.accelerators
                                        .stale
                                        .then_some(host.accelerators.observed)
                                        .flatten(),
                                    cx,
                                ))
                            },
                        )
                        .child(GpuCards::header(reading, cx))
                        .when(reading.devices.len() > 1, |content| {
                            content.child(GpuCards::overview(reading, cx))
                        })
                        .child(self.cards.render_devices(reading, cx));
                } else if !matches!(host.accelerators.status, MetricStatus::Unsupported) {
                    content = content.child(status_row(
                        "accelerator",
                        &host.accelerators.status,
                        None,
                        cx,
                    ));
                }
                if host.probe_status != MetricStatus::Ready {
                    content = content.child(status_row("probe", &host.probe_status, None, cx));
                }
                content = content.children(super::hardware_cards::render(
                    host,
                    &self.details,
                    &self.dispatch,
                    cx,
                ));
                if host.hardware.values().all(|metric| {
                    metric.value.as_ref().is_none_or(Vec::is_empty)
                        && matches!(metric.status, MetricStatus::Ready | MetricStatus::Loading)
                }) && host.accelerators.status == MetricStatus::Unsupported
                    && host.probe_status == MetricStatus::Ready
                {
                    content = content.child(
                        div()
                            .id("monitor-no-hardware")
                            .test_support()
                            .aria_label(t!("monitor.no_hardware"))
                            .text_sm()
                            .child(t!("monitor.no_hardware")),
                    );
                }
            } else if self.gpu_processes {
                content = content.child(status_row(
                    "accelerator",
                    &host.accelerators.status,
                    host.accelerators
                        .stale
                        .then_some(host.accelerators.observed)
                        .flatten(),
                    cx,
                ));
                if let Some(reading) = &host.accelerators.value {
                    content = content.child(self.cards.render_processes(reading, true, cx));
                }
            }
        } else {
            content = content.child(
                div()
                    .id("monitor-loading")
                    .test_support()
                    .child(t!("tools.reading")),
            );
        }
        let mut root = v_flex()
            .id("monitor")
            .test_support()
            .track_focus(&self.focus_handle)
            .size_full()
            .min_w_0()
            .child(header);
        if page == MonitorPage::Processes {
            let dispatch = self.dispatch.clone();
            root = root.child(
                div().p_3().child(
                    SegmentedControl::new("monitor-process-source")
                        .small()
                        .selected_index(Some(usize::from(self.gpu_processes)))
                        .segments([
                            Segment::new(t!("monitor.system.title")),
                            Segment::new("GPU/NPU"),
                        ])
                        .on_change(move |ix, window, cx| {
                            dispatch.dispatch_action(&SelectMonitorProcesses(*ix == 1), window, cx)
                        }),
                ),
            );
            if !self.gpu_processes {
                return root.child(div().flex_1().min_h_0().child(self.processes.clone()));
            }
        }
        if page == MonitorPage::Processes {
            return root.child(content.flex_1().min_h_0());
        }
        root.child(
            div()
                .flex_1()
                .min_h_0()
                .child(div().size_full().overflow_y_scrollbar().child(content)),
        )
    }
}

pub(super) fn status_text(status: &MetricStatus) -> SharedString {
    match status {
        MetricStatus::Loading => t!("tools.reading"),
        MetricStatus::Ready => t!("monitor.state.ready"),
        MetricStatus::Unsupported => t!("monitor.state.unsupported"),
        MetricStatus::PermissionDenied => t!("monitor.state.denied"),
        MetricStatus::SudoDenied => t!("monitor.state.sudo_denied"),
        MetricStatus::SudoUnavailable => t!("monitor.state.sudo_unavailable"),
        MetricStatus::Unavailable => t!("monitor.state.unavailable"),
        MetricStatus::Timeout => t!("monitor.state.timeout"),
        MetricStatus::Failed(_) => t!("monitor.state.failed"),
    }
}
pub(super) fn status_row(
    domain: &str,
    status: &MetricStatus,
    stale: Option<std::time::Instant>,
    cx: &App,
) -> impl IntoElement {
    let current = if domain == "system" && *status == MetricStatus::Timeout {
        t!("monitor.state.retrying_timeout")
    } else {
        status_text(status)
    };
    let text = if let Some(at) = stale {
        t!(
            "monitor.state.age",
            seconds = cx
                .background_executor()
                .now()
                .saturating_duration_since(at)
                .as_secs(),
            status = current
        )
    } else if *status == MetricStatus::Ready {
        SharedString::default()
    } else {
        current
    };
    div()
        .id(SharedString::from(if domain == "system" {
            "monitor-message".into()
        } else {
            format!("monitor-status-{domain}")
        }))
        .test_support()
        .aria_label(text.clone())
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(text)
}
/// A card's quiet note at the end of its title row: 「1 核」.
fn note(text: SharedString, cx: &App) -> AnyElement {
    div()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(text)
        .into_any_element()
}

/// In place of the note, when the card folds something away: 「8 核 ⌄」.
fn unfold_button(
    id: &'static str,
    label: SharedString,
    detail: MonitorDetail,
    unfolded: bool,
    tooltip: SharedString,
    dispatch: &FocusHandle,
) -> AnyElement {
    let dispatch = dispatch.clone();
    Button::new(id)
        .ghost()
        .xsmall()
        .label(label)
        .child(Icon::new(if unfolded {
            IconName::ChevronUp
        } else {
            IconName::ChevronDown
        }))
        .toggled(unfolded)
        .tooltip(tooltip)
        .on_click(move |_, window, cx| {
            dispatch.dispatch_action(&ToggleMonitorDetail(detail), window, cx)
        })
        .into_any_element()
}

fn render_system(system: &SystemInfo, uptime: Option<Duration>, cx: &App) -> impl IntoElement {
    let field = |id: &'static str, label: SharedString, value: String| {
        v_flex()
            .min_w_0()
            .gap_0p5()
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(label),
            )
            .child(
                div()
                    .id(id)
                    .test_support()
                    .aria_label(value.clone())
                    .text_sm()
                    .child(value),
            )
    };
    card(
        "monitor-system",
        CatalogIcon::Monitor,
        t!("monitor.system.title"),
        None,
        cx,
    )
    .child(
        div()
            .grid()
            .grid_cols(2)
            .gap_x_4()
            .gap_y_3()
            .child(field(
                "monitor-host",
                t!("monitor.system.host"),
                system.host_name.clone(),
            ))
            .child(field(
                "monitor-arch",
                t!("monitor.system.arch"),
                system.arch.clone(),
            ))
            .child(field(
                "monitor-os",
                t!("monitor.system.os"),
                system.os.clone(),
            ))
            .child(field(
                "monitor-uptime",
                t!("monitor.system.uptime"),
                uptime.map(format_duration).unwrap_or_default(),
            )),
    )
}

/// A ring with the share in its middle, as big as the eye needs to catch a
/// change across the room.
/// How the per-core bars lay out this frame.
struct CoresLayout {
    /// Every row, not only the first.
    unfolded: bool,
    /// How many bars make a row, from last frame's width; `None` until the
    /// row has been laid out once.
    per_row: Option<usize>,
    /// Where the row records its width for the next frame.
    width: Rc<Cell<Option<Pixels>>>,
    /// The panel to redraw when that width changes.
    panel: EntityId,
}

fn render_cpu(
    system: Option<&SystemInfo>,
    snapshot: &Snapshot,
    layout: &CoresLayout,
    dispatch: &FocusHandle,
    cx: &App,
) -> impl IntoElement {
    let model = system
        .map(|system| system.cpu_model.clone())
        .filter(|model| !model.is_empty());
    let total = snapshot.cpu.as_ref().map(|load| load.total);
    let cores = snapshot
        .cpu
        .as_ref()
        .map(|load| load.cores.as_slice())
        .filter(|cores| cores.len() > 1);
    // A host whose cores fill more than the one row shown can unfold the
    // rest; most hosts fit, and have no button.
    let more = layout
        .per_row
        .filter(|per_row| cores.is_some_and(|cores| cores.len() > *per_row));
    let count = tn!("monitor.cpu.cores", snapshot.cores);
    let meta = if more.is_some() {
        unfold_button(
            "monitor-cores-toggle",
            count,
            MonitorDetail::Cores,
            layout.unfolded,
            if layout.unfolded {
                t!("monitor.cpu.fold_cores")
            } else {
                t!("monitor.cpu.unfold_cores")
            },
            dispatch,
        )
    } else {
        note(count, cx)
    };
    let shown = cores.map(|cores| match more {
        Some(per_row) if !layout.unfolded => &cores[..per_row],
        _ => cores,
    });
    card("monitor-cpu", IconName::Cpu, "CPU", Some(meta), cx)
        .when_some(model, |card, model| {
            card.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(model),
            )
        })
        .child(
            h_flex()
                .gap_4()
                .child(gauge("monitor-cpu-gauge", total, cx))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_3()
                        .child(meter(
                            "monitor-cpu-usage",
                            t!("monitor.cpu.average"),
                            total,
                            cx,
                        ))
                        .when_some(shown, |column, shown| {
                            column.child(render_cores(shown, layout, cx))
                        }),
                ),
        )
}

/// The cores' loads as a column each, wrapping onto more rows the more
/// there are: 64 take three rows at the sidebar's usual width. Folded, only
/// the first row; it measures its width for the next frame to know how many
/// make one. Pointing at a bar tells its core's number and share.
fn render_cores(cores: &[Option<f32>], layout: &CoresLayout, cx: &App) -> impl IntoElement {
    let core = |index: usize, load: Option<f32>| {
        t!(
            "monitor.cpu.core",
            index = index,
            load = load.map(format_percent).unwrap_or_else(|| "—".into())
        )
    };
    let label = cores
        .iter()
        .enumerate()
        .map(|(index, load)| core(index, *load))
        .reduce(|list, next| t!("monitor.cpu.core_list", list = list, core = next))
        .unwrap_or_default();
    let (width, panel) = (layout.width.clone(), layout.panel);
    h_flex()
        .id("monitor-cpu-cores")
        .test_support()
        .aria_label(label)
        .relative()
        .w_full()
        .flex_wrap()
        .gap_0p5()
        // Before the first measurement, a row's height keeps the rest out.
        .when(!layout.unfolded, |row| {
            row.max_h(rems(1.5)).overflow_hidden()
        })
        .children(cores.iter().enumerate().map(|(index, load)| {
            let color = load.map_or(cx.theme().muted_foreground, |value| load_color(value, cx));
            let tooltip = core(index, *load);
            div()
                .id(("monitor-core", index as u64))
                .test_support()
                .relative()
                .w(rems(0.375))
                .h(rems(1.5))
                .rounded_sm()
                .bg(color.opacity(0.2))
                .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
                .child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .w_full()
                        .h(relative(load.unwrap_or(0.).clamp(0., 100.) / 100.))
                        .rounded_sm()
                        .bg(color),
                )
        }))
        .child(
            canvas(
                move |bounds, window, _| {
                    if width.get() != Some(bounds.size.width) {
                        width.set(Some(bounds.size.width));
                        window.on_next_frame(move |_, cx| cx.notify(panel));
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        )
}

/// 「421.02 MB / 973.25 MB」, the total quieter.
fn render_memory(memory: &Memory, cx: &App) -> impl IntoElement {
    card(
        "monitor-memory",
        IconName::MemoryStick,
        t!("monitor.memory.title"),
        None,
        cx,
    )
    .child(
        h_flex()
            .gap_4()
            .child(gauge("monitor-memory-gauge", Some(memory.percent()), cx))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_3()
                    .child(
                        v_flex()
                            .gap_1p5()
                            .child(meter(
                                "monitor-memory-usage",
                                t!("monitor.memory.physical"),
                                Some(memory.percent()),
                                cx,
                            ))
                            .child(amount(
                                "monitor-memory-amount",
                                memory.used,
                                memory.total,
                                cx,
                            )),
                    )
                    .when(memory.swap_total > 0, |column| {
                        column.child(
                            v_flex()
                                .gap_1p5()
                                .child(meter(
                                    "monitor-swap-usage",
                                    t!("monitor.memory.swap"),
                                    Some(memory.swap_percent()),
                                    cx,
                                ))
                                .child(amount(
                                    "monitor-swap-amount",
                                    memory.swap_used,
                                    memory.swap_total,
                                    cx,
                                )),
                        )
                    }),
            ),
    )
}

fn render_network(
    snapshot: &Snapshot,
    unfolded: bool,
    dispatch: &FocusHandle,
    cx: &App,
) -> impl IntoElement {
    let busy = snapshot.busy_interfaces();
    let meta = (busy.len() > 1).then(|| {
        unfold_button(
            "monitor-interfaces-toggle",
            tn!("monitor.network.interfaces", busy.len()),
            MonitorDetail::Interfaces,
            unfolded,
            if unfolded {
                t!("monitor.network.fold")
            } else {
                t!("monitor.network.unfold")
            },
            dispatch,
        )
    });
    let shown: Vec<&InterfaceRate> = if unfolded {
        busy
    } else {
        snapshot.main_interface().into_iter().collect()
    };
    let rate = |rate: Option<f64>| rate.map_or("—".into(), format_rate);
    card(
        "monitor-network",
        IconName::Network,
        t!("monitor.network.title"),
        meta,
        cx,
    )
    .when(shown.is_empty(), |card| {
        card.child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(t!("monitor.network.none")),
        )
    })
    .children(shown.into_iter().map(|interface| {
        let (up, down) = (rate(interface.transmitted), rate(interface.received));
        h_flex()
            .id(SharedString::from(format!(
                "monitor-interface:{}",
                interface.name
            )))
            .test_support()
            .aria_label(t!("monitor.network.label", up = up, down = down))
            .gap_3()
            .text_sm()
            .child(
                numeric(interface.name.clone(), cx)
                    .flex_1()
                    .min_w_0()
                    .truncate(),
            )
            .child(
                h_flex()
                    .flex_shrink_0()
                    .gap_3()
                    .text_xs()
                    .child(direction(IconName::ArrowUp, up, cx))
                    .child(direction(IconName::ArrowDown, down, cx)),
            )
    }))
}

fn direction(icon: IconName, rate: String, cx: &App) -> impl IntoElement {
    h_flex()
        .gap_0p5()
        .child(
            Icon::new(icon)
                .xsmall()
                .text_color(cx.theme().muted_foreground),
        )
        .child(numeric(rate, cx))
}

fn render_disks(disks: &[DiskUsage], cx: &App) -> impl IntoElement {
    card(
        "monitor-disks",
        IconName::HardDrive,
        t!("monitor.disks.title"),
        None,
        cx,
    )
    .when(disks.is_empty(), |card| {
        card.child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(t!("monitor.disks.none")),
        )
    })
    .children(disks.iter().map(|disk| {
        let percent = disk.percent();
        let color = load_color(percent, cx);
        let (used, total) = (format_bytes(disk.used), format_bytes(disk.total));
        v_flex()
            .id(SharedString::from(format!("monitor-disk:{}", disk.mount)))
            .test_support()
            .aria_label(t!(
                "monitor.disks.label",
                percent = format_percent(percent),
                used = used,
                total = total
            ))
            .gap_1p5()
            .child(
                h_flex()
                    .gap_3()
                    .text_sm()
                    .child(numeric(disk.mount.clone(), cx).flex_1().min_w_0())
                    .child(
                        numeric(format_percent(percent), cx)
                            .flex_shrink_0()
                            .text_color(color)
                            .font_weight(FontWeight::SEMIBOLD),
                    ),
            )
            .child(
                Progress::new(SharedString::from(format!(
                    "monitor-disk-bar:{}",
                    disk.mount
                )))
                .small()
                .value(percent)
                .color(color),
            )
            .child(
                numeric(format!("{used} / {total}"), cx)
                    .text_xs()
                    .text_color(cx.theme().muted_foreground),
            )
    }))
}
