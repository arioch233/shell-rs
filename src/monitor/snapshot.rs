//! 与界面无关的监控状态和硬件读数

use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CollectorDomain {
    Probe,
    System,
    Disks,
    Processes,
    Accelerator,
    Rga,
    Npu,
    Mpp,
    Dvpp,
    Tegra,
    Devfreq,
    Thermal,
    Psi,
}

impl CollectorDomain {
    pub const ALL: [Self; 13] = [
        Self::Probe,
        Self::System,
        Self::Disks,
        Self::Processes,
        Self::Accelerator,
        Self::Rga,
        Self::Npu,
        Self::Mpp,
        Self::Dvpp,
        Self::Tegra,
        Self::Devfreq,
        Self::Thermal,
        Self::Psi,
    ];
    pub fn key(self) -> &'static str {
        match self {
            Self::Probe => "probe",
            Self::System => "system",
            Self::Disks => "disks",
            Self::Processes => "processes",
            Self::Accelerator => "accelerator",
            Self::Rga => "rga",
            Self::Npu => "npu",
            Self::Mpp => "mpp",
            Self::Dvpp => "dvpp",
            Self::Tegra => "tegra",
            Self::Devfreq => "devfreq",
            Self::Thermal => "thermal",
            Self::Psi => "psi",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum MetricStatus {
    #[default]
    Loading,
    Ready,
    Unsupported,
    PermissionDenied,
    SudoDenied,
    SudoUnavailable,
    Unavailable,
    Timeout,
    Failed(String),
}

/// 错误状态与最后成功值分开，故障时仍能标识历史读数
#[derive(Clone, Debug)]
pub(crate) struct Metric<T> {
    pub value: Option<T>,
    pub status: MetricStatus,
    pub observed: Option<Instant>,
    pub stale: bool,
}

impl<T> Default for Metric<T> {
    fn default() -> Self {
        Self {
            value: None,
            status: MetricStatus::Loading,
            observed: None,
            stale: false,
        }
    }
}

impl<T> Metric<T> {
    pub fn record(&mut self, value: T, now: Instant) {
        self.value = Some(value);
        self.observed = Some(now);
        self.status = MetricStatus::Ready;
        self.stale = false;
    }
    pub fn fail(&mut self, status: MetricStatus) {
        self.status = status;
        self.stale = self.value.is_some();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unit {
    Percent,
    Hertz,
    Celsius,
    Text,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HardwareMetric {
    pub id: String,
    pub label: String,
    pub value: Option<f64>,
    pub unit: Unit,
    pub status: MetricStatus,
    pub source: String,
    pub detail: String,
}

impl HardwareMetric {
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        value: f64,
        unit: Unit,
        source: &str,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            value: Some(value),
            unit,
            status: MetricStatus::Ready,
            source: source.into(),
            detail: String::new(),
        }
    }
    pub fn state(source: &str, status: MetricStatus, detail: impl Into<String>) -> Self {
        Self {
            id: source.into(),
            label: source.rsplit('/').next().unwrap_or(source).into(),
            value: None,
            unit: Unit::Text,
            status,
            source: source.into(),
            detail: detail.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum MonitorPage {
    #[default]
    Overview,
    Hardware,
    Processes,
}

/// 保存展示偏好和本主机的读取授权选择，采样数据不进入设置文件
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct MonitorPreferences {
    pub allow_sudo: bool,
    pub page: MonitorPage,
    pub gpu_processes: bool,
    pub cores: bool,
    pub interfaces: bool,
    pub details: std::collections::HashSet<String>,
    pub gpu_all: Option<bool>,
    pub gpu_toggled: std::collections::HashSet<String>,
}
