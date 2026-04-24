use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubstepProgress {
    pub current: usize,
    pub total: usize,
    pub label: String,
}

impl SubstepProgress {
    pub fn fraction(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            self.current as f32 / self.total as f32
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerProgress {
    pub worker: usize,
    pub completed: usize,
    pub total: usize,
}

impl WorkerProgress {
    pub fn fraction(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            self.completed as f32 / self.total as f32
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressUpdate {
    pub step: usize,
    pub total_steps: usize,
    pub stage: String,
    pub detail: String,
    pub substep: Option<SubstepProgress>,
    pub workers: Vec<WorkerProgress>,
}

impl ProgressUpdate {
    pub fn new(
        step: usize,
        total_steps: usize,
        stage: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            step,
            total_steps,
            stage: stage.into(),
            detail: detail.into(),
            substep: None,
            workers: Vec::new(),
        }
    }

    pub fn with_substep(
        mut self,
        current: usize,
        total: usize,
        label: impl Into<String>,
    ) -> Self {
        self.substep = Some(SubstepProgress {
            current,
            total,
            label: label.into(),
        });
        self
    }

    pub fn with_workers(mut self, workers: Vec<WorkerProgress>) -> Self {
        self.workers = workers;
        self
    }

    pub fn fraction(&self) -> f32 {
        if self.total_steps == 0 {
            return 0.0;
        }

        let completed_steps = self.step.saturating_sub(1) as f32;
        let substep_fraction = self
            .substep
            .as_ref()
            .map(SubstepProgress::fraction)
            .unwrap_or(0.0);

        ((completed_steps + substep_fraction) / self.total_steps as f32).clamp(0.0, 1.0)
    }
}

#[derive(Clone)]
pub struct ProgressReporter {
    emit: Arc<dyn Fn(ProgressUpdate) + Send + Sync>,
}

impl ProgressReporter {
    pub fn new<F>(emit: F) -> Self
    where
        F: Fn(ProgressUpdate) + Send + Sync + 'static,
    {
        Self {
            emit: Arc::new(emit),
        }
    }

    pub fn emit(&self, update: ProgressUpdate) {
        (self.emit)(update);
    }

    pub fn step(
        &self,
        step: usize,
        total_steps: usize,
        stage: impl Into<String>,
        detail: impl Into<String>,
    ) {
        self.emit(ProgressUpdate::new(step, total_steps, stage, detail));
    }

    pub fn substep(
        &self,
        step: usize,
        total_steps: usize,
        stage: impl Into<String>,
        substep_current: usize,
        substep_total: usize,
        substep_label: impl Into<String>,
        detail: impl Into<String>,
    ) {
        self.emit(
            ProgressUpdate::new(step, total_steps, stage, detail).with_substep(
                substep_current,
                substep_total,
                substep_label,
            ),
        );
    }

    pub fn parallel_substep(
        &self,
        spec: ParallelProgressSpec,
    ) -> ParallelProgressTracker {
        let worker_count = rayon::current_num_threads().max(1);
        let tracker = ParallelProgressTracker {
            inner: Arc::new(ParallelProgressState {
                reporter: self.clone(),
                spec,
                worker_progress: (0..worker_count).map(|_| AtomicUsize::new(0)).collect(),
                total_completed: AtomicUsize::new(0),
                last_bucket: AtomicUsize::new(0),
            }),
        };
        tracker.emit_snapshot(0);
        tracker
    }
}

#[derive(Debug, Clone)]
pub struct ParallelProgressSpec {
    pub step: usize,
    pub total_steps: usize,
    pub stage: String,
    pub substep_current: usize,
    pub substep_total: usize,
    pub substep_label: String,
    pub detail: String,
    pub total_units: usize,
    pub unit_label: String,
}

#[derive(Clone)]
pub struct ParallelProgressTracker {
    inner: Arc<ParallelProgressState>,
}

struct ParallelProgressState {
    reporter: ProgressReporter,
    spec: ParallelProgressSpec,
    worker_progress: Vec<AtomicUsize>,
    total_completed: AtomicUsize,
    last_bucket: AtomicUsize,
}

impl ParallelProgressTracker {
    pub fn advance(&self, units: usize) {
        if self.inner.spec.total_units == 0 || units == 0 {
            return;
        }

        let worker_index = rayon::current_thread_index()
            .unwrap_or(0)
            .min(self.inner.worker_progress.len().saturating_sub(1));
        self.inner.worker_progress[worker_index].fetch_add(units, Ordering::Relaxed);

        let previous = self.inner.total_completed.fetch_add(units, Ordering::Relaxed);
        let completed = (previous + units).min(self.inner.spec.total_units);
        let bucket = completed.saturating_mul(100) / self.inner.spec.total_units.max(1);
        let last_bucket = self.inner.last_bucket.load(Ordering::Relaxed);
        if bucket > last_bucket
            && self
                .inner
                .last_bucket
                .compare_exchange(last_bucket, bucket, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
        {
            self.emit_snapshot(completed);
        }
    }

    pub fn finish(&self) {
        self.emit_snapshot(self.inner.spec.total_units);
    }

    fn emit_snapshot(&self, completed: usize) {
        let spec = &self.inner.spec;
        let workers = self
            .inner
            .worker_progress
            .iter()
            .enumerate()
            .map(|(index, progress)| WorkerProgress {
                worker: index + 1,
                completed: progress.load(Ordering::Relaxed).min(spec.total_units),
                total: spec.total_units,
            })
            .collect();

        self.inner.reporter.emit(
            ProgressUpdate::new(
                spec.step,
                spec.total_steps,
                spec.stage.clone(),
                format!(
                    "{} ({}/{}) {}",
                    spec.detail, completed, spec.total_units, spec.unit_label
                ),
            )
            .with_substep(
                spec.substep_current,
                spec.substep_total,
                spec.substep_label.clone(),
            )
            .with_workers(workers),
        );
    }
}
