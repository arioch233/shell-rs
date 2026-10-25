//! Jetson 媒体引擎逐来源快照，电源驻留时间与硬件忙碌率独立保存

use super::snapshot::{Metric, MetricStatus};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

#[derive(Default)]
pub(crate) struct TegraEngine {
    pub sources: BTreeMap<String, Metric<String>>,
    pub awake: Metric<f64>,
    baseline: Option<(u64, u64, Instant)>,
}

impl TegraEngine {
    pub fn current(&self, key: &str) -> Option<&str> {
        self.sources
            .get(key)
            .filter(|m| m.status == MetricStatus::Ready && !m.stale)?
            .value
            .as_deref()
    }

    pub fn update_awake(&mut self, now: Instant) {
        let counters = self
            .current("device/power/runtime_active_time")
            .and_then(|s| s.parse::<u64>().ok())
            .zip(
                self.current("device/power/runtime_suspended_time")
                    .and_then(|s| s.parse::<u64>().ok()),
            );
        let previous = self.baseline.take();
        self.awake.fail(MetricStatus::Unavailable);
        let Some((active, suspended)) = counters else {
            return;
        };
        self.baseline = Some((active, suspended, now));
        let Some((old_active, old_suspended, at)) = previous else {
            return;
        };
        let elapsed = now.saturating_duration_since(at);
        if elapsed.is_zero() || elapsed > Duration::from_secs(30) {
            return;
        }
        let Some(da) = active.checked_sub(old_active) else {
            return;
        };
        let Some(ds) = suspended.checked_sub(old_suspended) else {
            return;
        };
        let Some(total) = da.checked_add(ds).filter(|n| *n > 0) else {
            return;
        };
        self.awake.record(100. * da as f64 / total as f64, now);
    }

    fn stale(&mut self) {
        self.baseline = None;
        self.awake.stale = self.awake.value.is_some();
        for metric in self.sources.values_mut() {
            metric.stale = metric.value.is_some();
        }
    }
}

#[derive(Default)]
pub(crate) struct TegraTracker {
    pub engines: BTreeMap<&'static str, TegraEngine>,
}

impl TegraTracker {
    pub fn stale(&mut self) {
        for engine in self.engines.values_mut() {
            engine.stale();
        }
    }
    pub fn fail(&mut self, status: MetricStatus) {
        self.stale();
        for engine in self.engines.values_mut() {
            engine.awake.fail(status.clone());
            for metric in engine.sources.values_mut() {
                if metric.status != MetricStatus::Unsupported {
                    metric.fail(status.clone());
                }
            }
        }
    }
}
