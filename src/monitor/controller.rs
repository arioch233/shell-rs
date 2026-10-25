//! 监控目标与采集生命周期，后台结果只由前台定时轮询接收

use super::{
    collectors::{self, DeviceCapabilities},
    linux::{self, Parsed, Parts},
    model::{DiskUsage, Snapshot, SystemInfo, Tracker},
    scheduler::SamplingScheduler,
    snapshot::{CollectorDomain, HardwareMetric, Metric, MetricStatus},
};
use crate::{
    gpu::{self, AcceleratorReading},
    terminal::{ExecFailure, ExecHandle, ExecOptions, ExecTarget, RemoteTerminalId},
};
use gpui_kit::*;
use std::{
    collections::{BTreeMap, HashMap},
    rc::Rc,
    time::{Duration, Instant},
};

pub(crate) struct HostMonitorSnapshot {
    pub generation: u64,
    allow_sudo: bool,
    pub system: Metric<Snapshot>,
    pub info: Option<SystemInfo>,
    pub disks: Metric<Vec<DiskUsage>>,
    pub accelerators: Metric<AcceleratorReading>,
    pub hardware: BTreeMap<CollectorDomain, Metric<Vec<HardwareMetric>>>,
    pub capabilities: DeviceCapabilities,
    pub mpp: super::mpp::MppTracker,
    pub tegra: super::tegra::TegraTracker,
    pub dvpp: super::dvpp::DvppTracker,
    pub processes: Metric<Rc<crate::processes::ProcessSnapshot>>,
    process_tracker: crate::processes::ProcessTracker,
    pub probe_status: MetricStatus,
    tracker: Tracker,
    scheduler: SamplingScheduler,
    pending: BTreeMap<CollectorDomain, ExecHandle>,
    connected: bool,
    view: WeakEntity<crate::terminal::TerminalView>,
}

impl HostMonitorSnapshot {
    fn new(
        generation: u64,
        now: Instant,
        view: WeakEntity<crate::terminal::TerminalView>,
        allow_sudo: bool,
    ) -> Self {
        Self {
            allow_sudo,
            generation,
            processes: Metric::default(),
            process_tracker: crate::processes::ProcessTracker::default(),
            system: Metric::default(),
            info: None,
            disks: Metric::default(),
            accelerators: Metric::default(),
            hardware: BTreeMap::new(),
            capabilities: DeviceCapabilities::default(),
            mpp: super::mpp::MppTracker::default(),
            dvpp: super::dvpp::DvppTracker::default(),
            tegra: super::tegra::TegraTracker::default(),
            probe_status: MetricStatus::Loading,
            tracker: Tracker::default(),
            scheduler: SamplingScheduler::new(now),
            pending: BTreeMap::new(),
            connected: true,
            view,
        }
    }
    fn stale(&mut self) {
        self.mpp.stale();
        self.dvpp.stale();
        self.tegra.stale();
        self.processes.stale = self.processes.value.is_some();
        self.system.stale = self.system.value.is_some();
        self.disks.stale = self.disks.value.is_some();
        self.accelerators.stale = self.accelerators.value.is_some();
        for metric in self.hardware.values_mut() {
            metric.stale = metric.value.is_some();
        }
    }
    fn reset(&mut self, generation: u64, now: Instant) {
        self.stale();
        self.generation = generation;
        self.pending.clear();
        self.process_tracker = crate::processes::ProcessTracker::default();
        self.tracker = Tracker::default();
        self.scheduler = SamplingScheduler::new(now);
        self.info = None;
        self.capabilities = DeviceCapabilities::default();
        self.probe_status = MetricStatus::Loading;
        self.connected = true;
    }
    fn fail(&mut self, domain: CollectorDomain, status: MetricStatus) {
        match domain {
            CollectorDomain::System => self.system.fail(status),
            CollectorDomain::Processes => self.processes.fail(status),
            CollectorDomain::Disks => self.disks.fail(status),
            CollectorDomain::Accelerator => self.accelerators.fail(status),
            CollectorDomain::Probe => self.probe_status = status,
            CollectorDomain::Tegra => {
                self.tegra.fail(status.clone());
                self.hardware.entry(domain).or_default().fail(status);
            }
            CollectorDomain::Dvpp => {
                self.dvpp.fail(status.clone());
                self.hardware.entry(domain).or_default().fail(status);
            }
            _ => self.hardware.entry(domain).or_default().fail(status),
        }
    }
}

pub(crate) struct MonitorController {
    target: Option<ExecTarget>,
    active: bool,
    allow_sudo: bool,
    processes_active: bool,
    hosts: HashMap<RemoteTerminalId, HostMonitorSnapshot>,
    _poll: Task<()>,
}

impl MonitorController {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let poll = cx.spawn(async move |this, cx| {
            loop {
                if this.update(cx, |this, cx| this.tick(cx)).is_err() {
                    break;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(50))
                    .await;
            }
        });
        Self {
            target: None,
            active: false,
            allow_sudo: false,
            processes_active: false,
            hosts: HashMap::new(),
            _poll: poll,
        }
    }
    pub fn set_processes_active(&mut self, active: bool, cx: &mut Context<Self>) {
        if self.processes_active == active {
            return;
        }
        self.processes_active = active;
        if active
            && let Some(host) = self
                .target
                .as_ref()
                .and_then(|target| self.hosts.get_mut(&target.terminal))
        {
            host.scheduler
                .refresh(CollectorDomain::Processes, cx.background_executor().now());
        }
    }
    pub fn target(&self) -> Option<ExecTarget> {
        self.target.clone()
    }
    pub fn snapshot(&self) -> Option<&HostMonitorSnapshot> {
        self.target
            .as_ref()
            .and_then(|target| self.hosts.get(&target.terminal))
    }
    pub fn retain(&mut self, terminals: &[RemoteTerminalId]) {
        self.hosts.retain(|id, _| terminals.contains(id));
    }
    pub fn set_allow_sudo(&mut self, allow_sudo: bool, cx: &mut Context<Self>) {
        self.set_target(self.target.clone(), self.active, allow_sudo, cx);
    }
    pub fn set_target(
        &mut self,
        target: Option<ExecTarget>,
        active: bool,
        allow_sudo: bool,
        cx: &mut Context<Self>,
    ) {
        let policy_changed = self.allow_sudo != allow_sudo;
        self.allow_sudo = allow_sudo;
        if self.target == target && self.active == active {
            if policy_changed {
                self.tick(cx);
                cx.notify();
            }
            return;
        }
        if let Some(host) = self
            .target
            .as_ref()
            .and_then(|target| self.hosts.get_mut(&target.terminal))
        {
            host.stale();
        }
        self.target = target;
        self.active = active;
        if let Some(host) = self
            .target
            .as_ref()
            .and_then(|target| self.hosts.get_mut(&target.terminal))
        {
            host.stale();
            host.tracker = Tracker::default();
            host.process_tracker = crate::processes::ProcessTracker::default();
            let now = cx.background_executor().now();
            for domain in CollectorDomain::ALL {
                if domain != CollectorDomain::Probe {
                    host.scheduler.refresh(domain, now);
                }
            }
        }
        self.tick(cx);
        cx.notify();
    }
    pub fn refresh_target(
        &mut self,
        terminal: RemoteTerminalId,
        generation: u64,
        domain: CollectorDomain,
        cx: &mut Context<Self>,
    ) {
        if let Some(host) = self
            .hosts
            .get_mut(&terminal)
            .filter(|host| host.generation == generation)
        {
            host.scheduler
                .refresh(domain, cx.background_executor().now());
        }
        cx.notify();
    }
    pub fn refresh(&mut self, domain: Option<CollectorDomain>, cx: &mut Context<Self>) {
        if !self.active {
            return;
        }
        if let Some(host) = self
            .target
            .as_ref()
            .and_then(|target| self.hosts.get_mut(&target.terminal))
        {
            let now = cx.background_executor().now();
            for item in CollectorDomain::ALL {
                if domain.is_none() || domain == Some(item) {
                    host.scheduler.refresh(item, now);
                }
            }
        }
        self.tick(cx);
        cx.notify();
    }
    fn tick(&mut self, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        let mut changed = false;
        // 已提交的请求完成后释放名额，隐藏期间不发新请求
        for (terminal, host) in &mut self.hosts {
            let Some(view) = host.view.upgrade() else {
                host.pending.clear();
                continue;
            };
            let generation = view.read(cx).generation(cx);
            if host.generation != generation {
                host.reset(generation, now);
                changed = true;
            }
            if !matches!(
                view.read(cx).lifecycle(cx),
                crate::terminal::TerminalLifecycle::Running
            ) {
                host.pending.clear();
                if host.connected {
                    host.connected = false;
                    host.stale();
                    host.system.fail(MetricStatus::Unavailable);
                    changed = true;
                }
                continue;
            }
            let answers = host
                .pending
                .iter()
                .filter_map(|(domain, handle)| handle.try_recv().map(|result| (*domain, result)))
                .collect::<Vec<_>>();
            for (domain, result) in answers {
                host.pending.remove(&domain);
                changed = true;
                let first = (domain == CollectorDomain::System && host.info.is_none())
                    || (domain == CollectorDomain::Processes
                        && (host.processes.value.is_none() || host.processes.stale));
                let received =
                    matches!(&result, Ok(output) if output.exit_code.is_none_or(|code| code == 0));
                let status = match result {
                    Ok(output) if output.exit_code.is_none_or(|code| code == 0) => {
                        Self::record(host, domain, &output.stdout, now)
                    }
                    Ok(_) => Err(MetricStatus::Failed("nonzero_exit".into())),
                    Err(error) => Err(match error {
                        ExecFailure::Timeout => MetricStatus::Timeout,
                        ExecFailure::Disconnected | ExecFailure::Cancelled => {
                            MetricStatus::Unavailable
                        }
                        ExecFailure::Unsupported => MetricStatus::Unsupported,
                        ExecFailure::OutputLimit => MetricStatus::Failed("output_limit".into()),
                        ExecFailure::Busy => MetricStatus::Failed("busy".into()),
                        ExecFailure::Transport(_) => MetricStatus::Failed("transport".into()),
                    }),
                };
                let timed_out = domain != CollectorDomain::System
                    && matches!(status, Err(MetricStatus::Timeout));
                host.scheduler
                    .finish(domain, now, status.is_err(), timed_out, first);
                if let Err(status) = status {
                    if domain == CollectorDomain::Mpp {
                        host.mpp.fail(status.clone(), now);
                    }
                    if !matches!(domain, CollectorDomain::Dvpp | CollectorDomain::Tegra)
                        || !received
                    {
                        host.fail(domain, status);
                    }
                }
                if domain == CollectorDomain::Probe {
                    host.scheduler.pause(domain);
                }
                if !self.active
                    || self
                        .target
                        .as_ref()
                        .is_none_or(|target| target.terminal != *terminal)
                {
                    host.stale();
                }
            }
        }
        let Some(target) = self.target.clone().filter(|_| self.active) else {
            if changed {
                cx.notify();
            }
            return;
        };
        let Some(view) = target.view.upgrade() else {
            self.hosts.remove(&target.terminal);
            return;
        };
        let generation = view.read(cx).generation(cx);
        let host = self.hosts.entry(target.terminal).or_insert_with(|| {
            HostMonitorSnapshot::new(generation, now, target.view.clone(), self.allow_sudo)
        });
        if host.generation != generation {
            host.reset(generation, now);
            changed = true;
        }
        if !matches!(
            view.read(cx).lifecycle(cx),
            crate::terminal::TerminalLifecycle::Running
        ) {
            if host.connected {
                host.connected = false;
                host.stale();
                host.system.fail(MetricStatus::Unavailable);
                changed = true;
            }
            if changed {
                cx.notify();
            }
            return;
        }
        if !host.connected {
            host.reset(generation, now);
            changed = true;
        }
        // 切换权限等待旧请求结束，不丢弃驱动超时状态，也不重复启动驱动读取
        let policy_domain = |domain: CollectorDomain| {
            matches!(
                domain,
                CollectorDomain::Probe
                    | CollectorDomain::Rga
                    | CollectorDomain::Npu
                    | CollectorDomain::Mpp
                    | CollectorDomain::Tegra
            )
        };
        if host.allow_sudo != self.allow_sudo && !host.pending.keys().copied().any(policy_domain) {
            host.allow_sudo = self.allow_sudo;
            host.capabilities = DeviceCapabilities::default();
            host.probe_status = MetricStatus::Loading;
            for domain in [
                CollectorDomain::Probe,
                CollectorDomain::Rga,
                CollectorDomain::Npu,
                CollectorDomain::Mpp,
                CollectorDomain::Tegra,
            ] {
                if domain == CollectorDomain::Tegra {
                    host.tegra.stale();
                }
                if domain == CollectorDomain::Mpp {
                    host.mpp.stale();
                }
                host.scheduler.refresh(domain, now);
                if let Some(metric) = host.hardware.get_mut(&domain) {
                    metric.stale = metric.value.is_some();
                }
            }
            changed = true;
        }
        // 系统先获得名额，能力检测和其他域使用另一个名额
        for domain in host.scheduler.order() {
            if host.allow_sudo != self.allow_sudo && policy_domain(domain) {
                continue;
            }
            if domain == CollectorDomain::Processes && !self.processes_active {
                continue;
            }
            if !matches!(
                domain,
                CollectorDomain::System
                    | CollectorDomain::Probe
                    | CollectorDomain::Disks
                    | CollectorDomain::Processes
                    | CollectorDomain::Accelerator
            ) && host.probe_status != MetricStatus::Ready
            {
                continue;
            }
            if !host.scheduler.start(domain, now) {
                continue;
            }
            let command = match domain {
                CollectorDomain::System => Some(linux::command(Parts {
                    system: host.info.is_none(),
                    disks: false,
                })),
                CollectorDomain::Disks => Some(linux::command(Parts {
                    system: false,
                    disks: true,
                })),
                CollectorDomain::Probe => Some(collectors::probe_command()),
                CollectorDomain::Accelerator => Some(gpu::collect_command()),
                CollectorDomain::Processes => Some(crate::processes::collect_command()),
                _ => collectors::command(&host.capabilities, domain, host.allow_sudo),
            };
            let Some(command) = command else {
                if !host.capabilities.supports(domain) {
                    host.hardware.remove(&domain);
                    host.scheduler.finish(domain, now, false, false, false);
                    host.scheduler.pause(domain);
                    changed = true;
                    continue;
                }
                if domain == CollectorDomain::Tegra {
                    let _ = collectors::tegra::collect(
                        "@@complete",
                        &host.capabilities,
                        &mut host.tegra,
                        now,
                    );
                }
                if domain == CollectorDomain::Dvpp {
                    let _ = collectors::dvpp::collect(
                        "@@complete",
                        &host.capabilities,
                        &mut host.dvpp,
                        now,
                    );
                }
                if domain == CollectorDomain::Mpp {
                    let _ = collectors::mpp::collect(
                        "@@complete",
                        &host.capabilities,
                        &mut host.mpp,
                        now,
                    );
                }
                let rows = collectors::parse_nodes("@@complete", &host.capabilities, domain)
                    .unwrap_or_default();
                if host.capabilities.soc.is_some()
                    || rows.iter().any(|row| row.status == MetricStatus::Ready)
                {
                    host.hardware.entry(domain).or_default().record(rows, now);
                }
                host.scheduler.finish(domain, now, false, false, false);
                host.scheduler.pause(domain);
                changed = true;
                continue;
            };
            let mut options = ExecOptions {
                label: Some(domain.key()),
                ..ExecOptions::default()
            };
            if !matches!(
                domain,
                CollectorDomain::System
                    | CollectorDomain::Disks
                    | CollectorDomain::Processes
                    | CollectorDomain::Accelerator
            ) {
                options.timeout = Duration::from_secs(5);
            }
            match view.read(cx).exec_detailed(command, options, cx) {
                Some(handle) => {
                    host.pending.insert(domain, handle);
                    host.connected = true;
                }
                None => {
                    host.scheduler.finish(domain, now, true, false, false);
                    host.fail(domain, MetricStatus::Unavailable);
                    if host.connected {
                        host.stale();
                        host.connected = false;
                        changed = true;
                    }
                }
            }
        }
        if changed {
            cx.notify();
        }
    }
    fn record(
        host: &mut HostMonitorSnapshot,
        domain: CollectorDomain,
        output: &str,
        now: Instant,
    ) -> Result<(), MetricStatus> {
        match domain {
            CollectorDomain::System | CollectorDomain::Disks => match linux::parse(output) {
                Parsed::Reading(mut reading) => {
                    if domain == CollectorDomain::Disks {
                        let disks = reading
                            .disks
                            .take()
                            .ok_or_else(|| MetricStatus::Failed("missing_disks".into()))?;
                        host.disks.record(disks, now);
                    } else {
                        if reading.memory.total == 0 {
                            return Err(MetricStatus::Failed("missing_memory".into()));
                        }
                        if let Some(info) = reading.system.take() {
                            host.info = Some(info);
                        }
                        host.system.record(host.tracker.update(*reading, now), now);
                    }
                }
                Parsed::Unsupported(_) => return Err(MetricStatus::Unsupported),
            },
            CollectorDomain::Processes => match crate::processes::parse_reading(output) {
                crate::processes::Collected::Reading(reading) => {
                    let snapshot = host
                        .process_tracker
                        .update(*reading, chrono::Utc::now().timestamp());
                    host.processes.record(Rc::new(snapshot), now);
                }
                crate::processes::Collected::Unsupported(_) => {
                    return Err(MetricStatus::Unsupported);
                }
            },
            CollectorDomain::Probe => {
                host.mpp.stale();
                host.capabilities = collectors::parse_probe(output)?;
                host.hardware
                    .retain(|domain, _| host.capabilities.supports(*domain));
                collectors::dvpp::probe(&host.capabilities, &mut host.dvpp);
                collectors::tegra::probe(&host.capabilities, &mut host.tegra);
                host.probe_status = MetricStatus::Ready;
                for domain in CollectorDomain::ALL {
                    if !matches!(
                        domain,
                        CollectorDomain::Probe
                            | CollectorDomain::System
                            | CollectorDomain::Disks
                            | CollectorDomain::Processes
                    ) {
                        host.scheduler.refresh(domain, now);
                    }
                }
            }
            CollectorDomain::Accelerator => match gpu::parse_reading(output) {
                gpu::Collected::NoTools => return Err(MetricStatus::Unsupported),
                gpu::Collected::Reading(reading) => {
                    let timeout = reading.timed_out;
                    let failed = !reading.errors.is_empty();
                    if !(timeout || failed) || host.accelerators.value.is_none() {
                        host.accelerators.record(*reading, now);
                    } else if let Some(previous) = &mut host.accelerators.value {
                        previous.errors = reading.errors;
                        previous.timed_out = timeout;
                    }
                    if timeout {
                        return Err(MetricStatus::Timeout);
                    }
                    if failed {
                        return Err(MetricStatus::Failed("accelerator_failed".into()));
                    }
                }
            },
            CollectorDomain::Tegra => {
                let result =
                    collectors::tegra::collect(output, &host.capabilities, &mut host.tegra, now);
                let metric = host.hardware.entry(domain).or_default();
                metric.record(Vec::new(), now);
                if let Err(status) = &result {
                    metric.fail(status.clone());
                }
                return result;
            }
            CollectorDomain::Dvpp => {
                let result =
                    collectors::dvpp::collect(output, &host.capabilities, &mut host.dvpp, now);
                host.hardware
                    .entry(domain)
                    .or_default()
                    .record(Vec::new(), now);
                // 部分节点失败时保留本轮其他节点的成功状态
                if let Err(status) = &result {
                    host.hardware
                        .entry(domain)
                        .or_default()
                        .fail(status.clone());
                }
                return result;
            }
            _ => {
                let mpp_result = if domain == CollectorDomain::Mpp {
                    collectors::mpp::collect(output, &host.capabilities, &mut host.mpp, now)
                } else {
                    Ok(())
                };
                let mut rows = collectors::parse_nodes(output, &host.capabilities, domain)?;
                let metric = host.hardware.entry(domain).or_default();
                if let Some(previous) = &metric.value {
                    rows = rows
                        .into_iter()
                        .flat_map(|row| {
                            let preserved: Vec<_> = previous
                                .iter()
                                .filter(|old| old.source == row.source && old.value.is_some())
                                .map(|old| {
                                    let mut old = old.clone();
                                    old.status = row.status.clone();
                                    old
                                })
                                .collect();
                            if row.status != MetricStatus::Ready && !preserved.is_empty() {
                                preserved
                            } else {
                                vec![row]
                            }
                        })
                        .collect();
                }
                let timeout = rows.iter().any(|row| row.status == MetricStatus::Timeout);
                let failed = rows.iter().any(|row| {
                    matches!(
                        row.status,
                        MetricStatus::Failed(_)
                            | MetricStatus::SudoDenied
                            | MetricStatus::SudoUnavailable
                    )
                });
                let access_failure = rows.iter().find_map(|row| {
                    matches!(
                        row.status,
                        MetricStatus::SudoDenied | MetricStatus::SudoUnavailable
                    )
                    .then(|| row.status.clone())
                });
                let observed = metric.observed;
                metric.record(rows, now);
                if timeout || failed {
                    metric.observed = observed.or(Some(now));
                }
                if timeout {
                    return Err(MetricStatus::Timeout);
                }
                if let Some(status) = access_failure {
                    return Err(status);
                }
                if failed {
                    return Err(MetricStatus::Failed("partial_failure".into()));
                }
                mpp_result?;
            }
        }
        Ok(())
    }
}
