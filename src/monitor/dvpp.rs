//! HiDVPP 只读通道快照，逐来源保留错误与最后成功读数

use std::{collections::BTreeMap, time::Instant};

use super::snapshot::{Metric, MetricStatus};

#[derive(Clone, Debug, Default)]
pub(crate) struct DvppChannel {
    pub id: u32,
    pub fields: BTreeMap<&'static str, String>,
}

#[derive(Default)]
pub(crate) struct DvppTracker {
    pub sources: BTreeMap<String, Metric<Vec<DvppChannel>>>,
}

impl DvppTracker {
    pub fn stale(&mut self) {
        for metric in self.sources.values_mut() {
            metric.stale = metric.value.is_some();
        }
    }

    pub fn fail(&mut self, status: MetricStatus) {
        for metric in self.sources.values_mut() {
            if metric.status != MetricStatus::Unsupported {
                metric.fail(status.clone());
            }
        }
    }

    pub fn record(
        &mut self,
        source: &str,
        result: Result<Vec<DvppChannel>, MetricStatus>,
        now: Instant,
    ) {
        let metric = self.sources.entry(source.into()).or_default();
        match result {
            Ok(channels) => metric.record(channels, now),
            Err(status) => metric.fail(status),
        }
    }
}
