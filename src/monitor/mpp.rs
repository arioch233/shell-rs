//! MPP 分来源快照及解码任务差分，生命周期归属当前主机连接

use std::{collections::BTreeMap, time::Instant};

use super::snapshot::{Metric, MetricStatus};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CodecKind {
    Encoder,
    Decoder,
    Other,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct MppSession {
    pub pid: u32,
    pub index: u64,
    pub device: String,
    pub kind: CodecKind,
    pub memory_bytes: Option<u64>,
    pub fields: BTreeMap<String, String>,
    pub fps: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct MppClock {
    pub name: String,
    pub core: String,
    pub rate_hz: u64,
    pub enable_count: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum MppReading {
    Sessions(Vec<MppSession>),
    Clocks(Vec<MppClock>),
    Tasks { index: u64, per_second: Option<f64> },
    Text(String),
    Diagnostic { bytes: usize, preview: String },
}

#[derive(Default)]
pub(super) struct MppTracker {
    pub sources: BTreeMap<String, Metric<MppReading>>,
    counters: BTreeMap<String, (u64, Instant)>,
}

impl MppTracker {
    pub fn record(
        &mut self,
        source: &str,
        reading: Result<MppReading, MetricStatus>,
        now: Instant,
    ) {
        let metric = self.sources.entry(source.into()).or_default();
        match reading {
            Ok(mut value) => {
                if let MppReading::Tasks { index, per_second } = &mut value {
                    *per_second = self.counters.insert(source.into(), (*index, now)).and_then(
                        |(previous, at)| {
                            let elapsed = now.saturating_duration_since(at).as_secs_f64();
                            // 长时间隐藏后不把跨窗口平均值当作实时速率
                            (elapsed > 0. && elapsed <= 30.)
                                .then_some(elapsed)
                                .and_then(|seconds| {
                                    index
                                        .checked_sub(previous)
                                        .map(|delta| delta as f64 / seconds)
                                })
                        },
                    );
                }
                metric.record(value, now);
            }
            Err(status) => {
                self.counters.remove(source);
                metric.fail(status);
            }
        }
    }

    pub fn stale(&mut self) {
        self.counters.clear();
        for metric in self.sources.values_mut() {
            metric.stale = metric.value.is_some();
        }
    }

    pub fn fail(&mut self, status: MetricStatus, now: Instant) {
        self.counters.clear();
        for metric in self.sources.values_mut() {
            if metric.observed != Some(now) && metric.status == MetricStatus::Ready {
                metric.fail(status.clone());
            }
        }
    }

    pub fn sessions(&self) -> Option<(&str, &Metric<MppReading>)> {
        // 两种文件名是别名，优先当前成功来源，不能重复计数
        [
            "/proc/mpp_service/sessions-summary",
            "/proc/mpp_service/sessions_summary",
        ]
        .into_iter()
        .filter_map(|path| self.sources.get(path).map(|metric| (path, metric)))
        .min_by_key(|(_, metric)| {
            (
                metric.stale || metric.status != MetricStatus::Ready,
                metric.value.is_none(),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn task_rates_reset_after_rollbacks_failures_gaps_and_connection_changes() {
        let mut tracker = MppTracker::default();
        let now = Instant::now();
        let path = "task";
        for (seconds, index, expected) in [
            (0, 100, None),
            (2, 150, Some(25.)),
            (4, 150, Some(0.)),
            (6, 10, None),
            (40, 40, None),
            (42, 80, Some(20.)),
        ] {
            tracker.record(
                path,
                Ok(MppReading::Tasks {
                    index,
                    per_second: None,
                }),
                now + Duration::from_secs(seconds),
            );
            assert_eq!(
                tracker.sources[path].value,
                Some(MppReading::Tasks {
                    index,
                    per_second: expected
                })
            );
        }
        tracker.record(
            path,
            Err(MetricStatus::PermissionDenied),
            now + Duration::from_secs(44),
        );
        assert!(tracker.sources[path].stale);
        tracker.record(
            path,
            Ok(MppReading::Tasks {
                index: 120,
                per_second: None,
            }),
            now + Duration::from_secs(46),
        );
        assert_eq!(
            tracker.sources[path].value,
            Some(MppReading::Tasks {
                index: 120,
                per_second: None
            })
        );
        tracker.stale();
        tracker.record(
            path,
            Ok(MppReading::Tasks {
                index: 200,
                per_second: None,
            }),
            now + Duration::from_secs(48),
        );
        assert_eq!(
            tracker.sources[path].value,
            Some(MppReading::Tasks {
                index: 200,
                per_second: None
            })
        );
    }

    #[test]
    fn sources_keep_independent_freshness_and_session_aliases_are_not_added() {
        let mut tracker = MppTracker::default();
        let now = Instant::now();
        let path = "/proc/mpp_service/sessions-summary";
        let alias = "/proc/mpp_service/sessions_summary";
        tracker.record(path, Ok(MppReading::Sessions(Vec::new())), now);
        tracker.record(alias, Err(MetricStatus::Unsupported), now);
        assert_eq!(tracker.sessions().unwrap().0, path);
        let next = now + Duration::from_secs(2);
        tracker.record("clock", Err(MetricStatus::Timeout), next);
        tracker.record(alias, Ok(MppReading::Sessions(Vec::new())), next);
        tracker.fail(MetricStatus::Timeout, next);
        assert!(tracker.sources[path].stale);
        assert!(!tracker.sources[alias].stale);
        assert_eq!(tracker.sessions().unwrap().0, alias);
        assert_eq!(tracker.sources["clock"].status, MetricStatus::Timeout);
    }
}
