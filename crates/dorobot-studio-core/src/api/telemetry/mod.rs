//! Robot-independent live telemetry and source status.

mod receiver;
mod wire;

pub use receiver::{ReceiverError, ZenohOptions, ZenohReceiver};
pub use wire::{Descriptor, JointUnit, JointValue, PoseFrame, SourceId, MAX_PAYLOAD};

use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

const STALE_AFTER: Duration = Duration::from_secs(1);
const MAX_SOURCES: usize = 64;

#[derive(Debug, Clone)]
pub struct ReceivedPose {
    pub frame: PoseFrame,
    pub received_at: Instant,
    pub missed_samples: u64,
    pub receive_hz: f64,
}

#[derive(Debug, Clone)]
pub struct RemoteSource {
    pub descriptor: Descriptor,
    pub latest: Option<ReceivedPose>,
}

#[derive(Debug, Default, Clone)]
pub struct SourceCatalog {
    sources: BTreeMap<SourceId, RemoteSource>,
    generation: u64,
    pub error: Option<String>,
    pub rejected: u64,
}

impl SourceCatalog {
    pub fn sources(&self) -> impl Iterator<Item = (&SourceId, &RemoteSource)> {
        self.sources.iter()
    }

    pub fn add_source(&mut self, descriptor: Descriptor) -> bool {
        if self.sources.contains_key(&descriptor.source) || self.sources.len() >= MAX_SOURCES {
            return false;
        }
        self.sources.insert(
            descriptor.source.clone(),
            RemoteSource {
                descriptor,
                latest: None,
            },
        );
        self.generation += 1;
        true
    }

    pub fn remove_source(&mut self, source: &SourceId) {
        if self.sources.remove(source).is_some() {
            self.generation += 1;
        }
    }

    pub fn accept_pose(&mut self, frame: PoseFrame, received_at: Instant) -> bool {
        let Some(source) = self.sources.get_mut(&frame.source) else {
            return false;
        };
        if frame.model_id != source.descriptor.model_id {
            return false;
        }
        let mut missed = 0;
        let mut hz = 0.0;
        if let Some(last) = &source.latest {
            if frame.sequence <= last.frame.sequence
                || frame.capture_monotonic_ns < last.frame.capture_monotonic_ns
            {
                return false;
            }
            missed = last
                .missed_samples
                .saturating_add(frame.sequence - last.frame.sequence - 1);
            let gap = received_at
                .saturating_duration_since(last.received_at)
                .as_secs_f64();
            hz = last.receive_hz;
            if gap > 0.0 {
                let instantaneous = 1.0 / gap;
                hz = if hz > 0.0 && gap < 1.0 {
                    hz * 0.9 + instantaneous * 0.1
                } else {
                    instantaneous
                };
            }
        }
        source.latest = Some(ReceivedPose {
            frame,
            received_at,
            missed_samples: missed,
            receive_hz: hz,
        });
        self.generation += 1;
        true
    }

    pub fn latest(&self, source: &SourceId) -> Option<&ReceivedPose> {
        self.sources.get(source)?.latest.as_ref()
    }

    pub fn is_stale(&self, source: &SourceId, now: Instant) -> bool {
        self.latest(source)
            .is_none_or(|pose| now.saturating_duration_since(pose.received_at) >= STALE_AFTER)
    }

    fn report_error(&mut self, error: Option<String>) {
        self.error = error;
        self.generation += 1;
    }
}
