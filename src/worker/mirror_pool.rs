use crate::mirrors::{MirrorEndpoint, MirrorKind};
use smallvec::SmallVec;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

type MirrorPlan = SmallVec<[MirrorEndpoint; 5]>;
type CoolingQueue = SmallVec<[(MirrorEndpoint, Instant); 5]>;

#[derive(Clone)]
pub struct MirrorPool {
    mirrors: Arc<Vec<MirrorEndpoint>>,
    penalties: Arc<Mutex<HashMap<MirrorKind, Instant>>>,
}

impl MirrorPool {
    pub fn new(mirrors: Vec<MirrorEndpoint>) -> Self {
        Self {
            mirrors: Arc::new(mirrors),
            penalties: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn is_single_mirror(&self) -> bool {
        self.mirrors.len() == 1
    }

    pub fn plan(&self) -> MirrorPlan {
        let now = Instant::now();
        let mut penalties = self.penalties.lock().unwrap();
        let mut ready: MirrorPlan = SmallVec::with_capacity(self.mirrors.len());
        let mut cooling: CoolingQueue = SmallVec::new();

        for mirror in self.mirrors.iter() {
            match penalties.get(&mirror.kind).copied() {
                Some(until) if until > now => cooling.push((mirror.clone(), until)),
                Some(_) => {
                    penalties.remove(&mirror.kind);
                    ready.push(mirror.clone());
                }
                None => ready.push(mirror.clone()),
            }
        }

        drop(penalties);

        if ready.is_empty() {
            cooling.sort_by_key(|(_, until)| *until);
            return cooling
                .into_iter()
                .map(|(mirror, _)| mirror)
                .collect::<MirrorPlan>();
        }

        cooling.sort_by_key(|(_, until)| *until);
        ready.extend(cooling.into_iter().map(|(mirror, _)| mirror));
        ready
    }

    pub fn single_mirror_cooldown(&self) -> Option<(MirrorEndpoint, Duration)> {
        if !self.is_single_mirror() {
            return None;
        }

        let mirror = self.mirrors.first()?.clone();
        let now = Instant::now();
        let penalties = self.penalties.lock().unwrap();

        penalties.get(&mirror.kind).and_then(|&until| {
            if until > now {
                Some((mirror, until - now))
            } else {
                None
            }
        })
    }

    pub fn mark_rate_limited(&self, kind: MirrorKind) {
        let mut penalties = self.penalties.lock().unwrap();
        penalties.insert(kind, Instant::now() + kind.rate_limit_backoff());
    }

    pub fn clear_penalty(&self, kind: MirrorKind) {
        let mut penalties = self.penalties.lock().unwrap();
        penalties.remove(&kind);
    }

    pub fn penalty_remaining(&self, kind: MirrorKind) -> Option<Duration> {
        let now = Instant::now();
        let penalties = self.penalties.lock().unwrap();
        penalties
            .get(&kind)
            .and_then(|&until| (until > now).then_some(until - now))
    }
}
