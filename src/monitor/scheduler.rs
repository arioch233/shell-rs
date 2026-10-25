//! 不依赖窗口的多域调度，系统域保留一个执行名额

use super::snapshot::CollectorDomain;
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

#[derive(Clone, Debug)]
struct Slot {
    due: Instant,
    in_flight: bool,
    paused: bool,
    unsafe_retry: bool,
    refresh: bool,
    failures: u32,
}

pub(crate) struct SamplingScheduler {
    slots: BTreeMap<CollectorDomain, Slot>,
}

impl SamplingScheduler {
    pub fn new(now: Instant) -> Self {
        Self {
            slots: CollectorDomain::ALL
                .into_iter()
                .map(|domain| {
                    (
                        domain,
                        Slot {
                            due: now,
                            in_flight: false,
                            paused: false,
                            unsafe_retry: false,
                            refresh: false,
                            failures: 0,
                        },
                    )
                })
                .collect(),
        }
    }
    pub fn start(&mut self, domain: CollectorDomain, now: Instant) -> bool {
        let other_busy = self
            .slots
            .iter()
            .any(|(key, slot)| *key != CollectorDomain::System && slot.in_flight);
        let slot = self.slots.get_mut(&domain).expect("known domain");
        if slot.in_flight
            || slot.paused
            || slot.due > now
            || (domain != CollectorDomain::System && other_busy)
        {
            return false;
        }
        slot.in_flight = true;
        true
    }
    pub fn finish(
        &mut self,
        domain: CollectorDomain,
        now: Instant,
        failed: bool,
        unsafe_retry: bool,
        first: bool,
    ) {
        let slot = self.slots.get_mut(&domain).expect("known domain");
        slot.in_flight = false;
        slot.unsafe_retry |= unsafe_retry;
        slot.failures = if failed {
            slot.failures.saturating_add(1)
        } else {
            0
        };
        slot.paused = unsafe_retry || (failed && domain != CollectorDomain::System);
        let seconds = if failed && domain == CollectorDomain::System {
            2_u64.saturating_pow(slot.failures.min(5)).min(30)
        } else {
            match domain {
                CollectorDomain::System if first => 1,
                CollectorDomain::System
                | CollectorDomain::Accelerator
                | CollectorDomain::Rga
                | CollectorDomain::Npu
                | CollectorDomain::Mpp => 2,
                CollectorDomain::Processes if first => 2,
                CollectorDomain::Processes => 15,
                CollectorDomain::Disks => 30,
                CollectorDomain::Probe => 86400,
                _ => 5,
            }
        };
        slot.due = if slot.refresh && !slot.unsafe_retry {
            now
        } else {
            now + Duration::from_secs(seconds)
        };
        slot.refresh = false;
    }
    pub fn refresh(&mut self, domain: CollectorDomain, now: Instant) {
        let slot = self.slots.get_mut(&domain).expect("known domain");
        if slot.unsafe_retry {
            return;
        }
        slot.paused = false;
        if slot.in_flight {
            slot.refresh = true;
        } else {
            slot.due = now;
        }
    }
    pub fn order(&self) -> Vec<CollectorDomain> {
        let mut domains: Vec<_> = self
            .slots
            .iter()
            .map(|(domain, slot)| (*domain, slot.due))
            .collect();
        domains.sort_by_key(|(domain, due)| (*domain != CollectorDomain::System, *due, *domain));
        domains.into_iter().map(|(domain, _)| domain).collect()
    }
    pub fn pause(&mut self, domain: CollectorDomain) {
        self.slots.get_mut(&domain).expect("known domain").paused = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_slow_hardware_domain_never_blocks_system_or_overlaps() {
        let now = Instant::now();
        let mut scheduler = SamplingScheduler::new(now);
        assert!(scheduler.start(CollectorDomain::Npu, now));
        assert!(!scheduler.start(CollectorDomain::Npu, now));
        assert!(!scheduler.start(CollectorDomain::Rga, now));
        assert!(scheduler.start(CollectorDomain::System, now));
        scheduler.refresh(CollectorDomain::Npu, now);
        assert!(!scheduler.start(CollectorDomain::Npu, now));
        scheduler.finish(CollectorDomain::Npu, now, true, true, false);
        scheduler.refresh(CollectorDomain::Npu, now);
        assert!(!scheduler.start(CollectorDomain::Npu, now + Duration::from_secs(60)));
        assert!(scheduler.start(CollectorDomain::Rga, now));
    }
}
