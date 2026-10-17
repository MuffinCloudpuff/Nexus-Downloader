use crate::core::resume::{ChunkManifest, ChunkStatus};
use std::collections::{HashMap, VecDeque};

pub const DEFAULT_MIN_SPLIT_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentFetchStatus {
    Completed,
    LeaseCompleted,
    Recoverable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentJob {
    pub index: usize,
    pub start: u64,
    pub end: u64,
    pub bytes_written: u64,
    pub attempts: u32,
}

impl SegmentJob {
    pub fn expected_size(&self) -> u64 {
        self.end - self.start + 1
    }

    pub fn completed_bytes(&self) -> u64 {
        self.expected_size().min(self.bytes_written)
    }

    pub fn next_offset(&self) -> u64 {
        self.start + self.completed_bytes()
    }

    pub fn remaining_bytes(&self) -> u64 {
        self.end
            .saturating_sub(self.next_offset())
            .saturating_add(1)
    }

    pub fn is_complete(&self) -> bool {
        self.completed_bytes() >= self.expected_size()
    }

    pub fn lease_end(&self, lease_bytes: u64) -> u64 {
        let next_offset = self.next_offset();
        let lease_bytes = lease_bytes.max(1);
        next_offset
            .saturating_add(lease_bytes.saturating_sub(1))
            .min(self.end)
    }
}

#[derive(Debug)]
pub struct SegmentQueue {
    pending: VecDeque<usize>,
    jobs: HashMap<usize, SegmentJob>,
    active: HashMap<usize, SegmentJob>,
    min_split_bytes: u64,
    next_index: usize,
}

impl SegmentQueue {
    pub fn from_chunks(chunks: &[ChunkManifest], min_split_bytes: u64) -> Self {
        let mut pending = VecDeque::new();
        let mut jobs = HashMap::new();
        let mut next_index = chunks.len();

        for chunk in chunks {
            let index = chunk.index as usize;
            next_index = next_index.max(index + 1);
            let job = SegmentJob {
                index,
                start: chunk.start,
                end: chunk.end,
                bytes_written: chunk.bytes_written.min(chunk.end - chunk.start + 1),
                attempts: 0,
            };
            if !job.is_complete() || chunk.status != ChunkStatus::Completed {
                pending.push_back(index);
                jobs.insert(index, job);
            }
        }

        Self {
            pending,
            jobs,
            active: HashMap::new(),
            min_split_bytes,
            next_index,
        }
    }

    pub fn pop_next(&mut self) -> Option<SegmentJob> {
        while let Some(index) = self.pending.pop_front() {
            let Some(job) = self.jobs.remove(&index) else {
                continue;
            };
            if job.is_complete() {
                continue;
            }
            self.active.insert(index, job.clone());
            return Some(job);
        }
        None
    }

    pub fn complete(&mut self, index: usize) {
        self.active.remove(&index);
        self.jobs.remove(&index);
    }

    pub fn requeue(&mut self, mut job: SegmentJob, bytes_written: u64) -> Option<SegmentJob> {
        self.active.remove(&job.index);
        job.bytes_written = bytes_written.min(job.expected_size());
        job.attempts = job.attempts.saturating_add(1);
        self.enqueue_incomplete(job)
    }

    pub fn complete_lease(
        &mut self,
        mut job: SegmentJob,
        bytes_written: u64,
    ) -> Option<SegmentJob> {
        self.active.remove(&job.index);
        job.bytes_written = bytes_written.min(job.expected_size());
        self.enqueue_incomplete(job)
    }

    pub fn update_active_progress(&mut self, index: usize, bytes_written: u64) {
        if let Some(job) = self.active.get_mut(&index) {
            job.bytes_written = bytes_written.min(job.expected_size());
        }
    }

    pub fn is_finished(&self) -> bool {
        self.pending.is_empty() && self.jobs.is_empty() && self.active.is_empty()
    }

    fn enqueue_incomplete(&mut self, mut job: SegmentJob) -> Option<SegmentJob> {
        if job.is_complete() {
            return None;
        }

        let split = self.split_requeued_job(&mut job);
        let job_index = job.index;
        self.jobs.insert(job_index, job);
        self.pending.push_back(job_index);
        if let Some(split) = split.clone() {
            self.jobs.insert(split.index, split.clone());
            self.pending.push_back(split.index);
        }
        split
    }

    fn split_requeued_job(&mut self, job: &mut SegmentJob) -> Option<SegmentJob> {
        let remaining = job.remaining_bytes();
        if remaining < self.min_split_bytes.saturating_mul(2) {
            return None;
        }

        let split_offset = job.next_offset() + remaining / 2;
        if split_offset == 0 || split_offset > job.end {
            return None;
        }

        let original_end = job.end;
        job.end = split_offset - 1;

        let new_index = self.next_index;
        self.next_index += 1;
        Some(SegmentJob {
            index: new_index,
            start: split_offset,
            end: original_end,
            bytes_written: 0,
            attempts: 0,
        })
    }
}

pub fn chunk_for_split(output_path: &str, job: &SegmentJob) -> ChunkManifest {
    ChunkManifest {
        index: job.index as u32,
        start: job.start,
        end: job.end,
        path: output_path.to_string(),
        status: ChunkStatus::Pending,
        bytes_written: job.bytes_written,
        checksum: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(index: u32, start: u64, end: u64, bytes_written: u64) -> ChunkManifest {
        ChunkManifest {
            index,
            start,
            end,
            path: "file.bin".to_string(),
            status: ChunkStatus::Pending,
            bytes_written,
            checksum: None,
        }
    }

    #[test]
    fn requeue_keeps_progress_pointer() {
        let chunks = vec![chunk(0, 0, 99, 0)];
        let mut queue = SegmentQueue::from_chunks(&chunks, 64);
        let job = queue.pop_next().unwrap();

        queue.requeue(job, 40);
        let resumed = queue.pop_next().unwrap();

        assert_eq!(resumed.next_offset(), 40);
        assert_eq!(resumed.remaining_bytes(), 60);
        assert_eq!(resumed.attempts, 1);
    }

    #[test]
    fn requeue_splits_large_remaining_tail() {
        let chunks = vec![chunk(0, 0, 99, 0)];
        let mut queue = SegmentQueue::from_chunks(&chunks, 16);
        let job = queue.pop_next().unwrap();

        let split = queue.requeue(job, 20).unwrap();
        let first = queue.pop_next().unwrap();
        let second = queue.pop_next().unwrap();

        assert_eq!(first.start, 0);
        assert_eq!(first.bytes_written, 20);
        assert_eq!(first.end + 1, split.start);
        assert_eq!(second.index, split.index);
        assert_eq!(second.end, 99);
    }

    #[test]
    fn completed_chunks_are_not_queued() {
        let mut done = chunk(0, 0, 99, 100);
        done.status = ChunkStatus::Completed;

        let queue = SegmentQueue::from_chunks(&[done], 16);

        assert!(queue.is_finished());
    }

    #[test]
    fn lease_end_caps_to_remaining_segment() {
        let job = SegmentJob {
            index: 0,
            start: 100,
            end: 199,
            bytes_written: 80,
            attempts: 0,
        };

        assert_eq!(job.next_offset(), 180);
        assert_eq!(job.lease_end(8), 187);
        assert_eq!(job.lease_end(64), 199);
    }

    #[test]
    fn completed_lease_requeues_and_splits_large_tail() {
        let chunks = vec![chunk(0, 0, 99, 0)];
        let mut queue = SegmentQueue::from_chunks(&chunks, 16);
        let job = queue.pop_next().unwrap();

        let split = queue.complete_lease(job, 20).unwrap();
        let first = queue.pop_next().unwrap();
        let second = queue.pop_next().unwrap();

        assert_eq!(first.bytes_written, 20);
        assert_eq!(first.end + 1, split.start);
        assert_eq!(second.index, split.index);
        assert_eq!(second.end, 99);
        assert_eq!(first.attempts, 0);
    }

    #[test]
    fn chunk_for_split_uses_new_segment_bounds() {
        let job = SegmentJob {
            index: 3,
            start: 50,
            end: 99,
            bytes_written: 0,
            attempts: 0,
        };

        let chunk = chunk_for_split("file.bin", &job);

        assert_eq!(chunk.index, 3);
        assert_eq!(chunk.start, 50);
        assert_eq!(chunk.end, 99);
        assert_eq!(chunk.bytes_written, 0);
        assert_eq!(chunk.status, ChunkStatus::Pending);
    }
}
