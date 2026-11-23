//! The jobs the server runs on its own threads, kept in memory only.

use std::sync::{Mutex, MutexGuard, PoisonError};

use super::wire::{Job, JobId, JobKind, JobState, Progress};

const KEEP_FINISHED: usize = 100;

pub struct Jobs {
    /// The Ctx's start time in ms, so ids from two runs never collide.
    start: u64,
    inner: Mutex<Inner>,
}

struct Inner {
    next: u64,
    jobs: Vec<Job>,
}

impl Jobs {
    pub fn new(start: u64) -> Jobs {
        Jobs { start, inner: Mutex::new(Inner { next: 1, jobs: Vec::new() }) }
    }

    /// Poison-tolerant: a job thread that panicked is still ended through here.
    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn start(&self, kind: JobKind, key: Option<String>, now: u64) -> JobId {
        let mut inner = self.inner();
        let id = JobId(format!("j{}-{}", self.start, inner.next));
        inner.next += 1;
        inner.jobs.push(Job {
            id: id.clone(),
            kind,
            state: JobState::Running,
            key,
            progress: None,
            started_at: now,
            ended_at: None,
            error: None,
        });
        id
    }

    pub fn get(&self, id: &JobId) -> Option<Job> {
        self.inner().jobs.iter().find(|j| j.id == *id).cloned()
    }

    pub fn running(&self) -> Vec<Job> {
        self.inner().jobs.iter().filter(|j| j.state == JobState::Running).cloned().collect()
    }

    pub fn progress(&self, id: &JobId, progress: Progress) {
        let mut inner = self.inner();
        if let Some(job) = inner.jobs.iter_mut().find(|j| j.id == *id && j.state == JobState::Running) {
            job.progress = Some(progress);
        }
    }

    /// Moves a running job to `state`. A job that already left Running keeps its first end state.
    pub fn finish(&self, id: &JobId, state: JobState, error: Option<String>, now: u64) {
        let mut inner = self.inner();
        let Some(job) = inner.jobs.iter_mut().find(|j| j.id == *id && j.state == JobState::Running) else { return };
        job.state = state;
        job.error = error;
        job.ended_at = Some(now);
        let finished = inner.jobs.iter().filter(|j| j.state != JobState::Running).count();
        if finished > KEEP_FINISHED {
            let oldest = inner
                .jobs
                .iter()
                .enumerate()
                .filter(|(_, j)| j.state != JobState::Running)
                .min_by_key(|(_, j)| j.ended_at)
                .map(|(i, _)| i);
            if let Some(i) = oldest {
                inner.jobs.remove(i);
            }
        }
    }
}
