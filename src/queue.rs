use crossbeam_channel::{bounded, Receiver, SendError, Sender};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Work item for the deletion queue
#[derive(Debug, Clone)]
pub enum FileJob {
    /// A regular file to delete
    File(Arc<Path>),
    /// A symbolic link to delete
    Symlink(Arc<Path>),
    /// An empty directory to delete (enqueued after all children)
    EmptyDir(Arc<Path>),
}

/// Adaptive bounded queue for coordinating between scanner and deleter threads
pub struct AdaptiveQueue {
    sender: Sender<FileJob>,
    receiver: Receiver<FileJob>,
    capacity: AtomicUsize,
    enqueued: Arc<AtomicUsize>,
    dequeued: Arc<AtomicUsize>,
}

impl AdaptiveQueue {
    /// Create a new adaptive queue with the given initial capacity
    pub fn new(initial_capacity: usize) -> Self {
        let (sender, receiver) = bounded(initial_capacity);

        Self {
            sender,
            receiver,
            capacity: AtomicUsize::new(initial_capacity),
            enqueued: Arc::new(AtomicUsize::new(0)),
            dequeued: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Send a job to the queue (blocking if full)
    pub fn send(&self, job: FileJob) -> Result<(), SendError<FileJob>> {
        self.enqueued.fetch_add(1, Ordering::Relaxed);
        self.sender.send(job)
    }

    /// Try to send a job without blocking
    pub fn try_send(&self, job: FileJob) -> Result<(), crossbeam_channel::TrySendError<FileJob>> {
        self.enqueued.fetch_add(1, Ordering::Relaxed);
        self.sender.try_send(job)
    }

    /// Receive a job from the queue (blocking)
    pub fn recv(&self) -> Result<FileJob, crossbeam_channel::RecvError> {
        let job = self.receiver.recv()?;
        self.dequeued.fetch_add(1, Ordering::Relaxed);
        Ok(job)
    }

    /// Try to receive a job without blocking
    pub fn try_recv(&self) -> Result<FileJob, crossbeam_channel::TryRecvError> {
        let job = self.receiver.try_recv()?;
        self.dequeued.fetch_add(1, Ordering::Relaxed);
        Ok(job)
    }

    /// Receive with a timeout
    pub fn recv_timeout(
        &self,
        timeout: std::time::Duration,
    ) -> Result<FileJob, crossbeam_channel::RecvTimeoutError> {
        let job = self.receiver.recv_timeout(timeout)?;
        self.dequeued.fetch_add(1, Ordering::Relaxed);
        Ok(job)
    }

    /// Get the current depth of the queue (enqueued - dequeued)
    pub fn depth(&self) -> usize {
        let enqueued = self.enqueued.load(Ordering::Relaxed);
        let dequeued = self.dequeued.load(Ordering::Relaxed);
        enqueued.saturating_sub(dequeued)
    }

    /// Get the current capacity
    pub fn capacity(&self) -> usize {
        self.capacity.load(Ordering::Relaxed)
    }

    /// Get a reference to the enqueued counter (for progress tracking)
    pub fn enqueued_counter(&self) -> Arc<AtomicUsize> {
        self.enqueued.clone()
    }

    /// Check if the queue is empty
    pub fn is_empty(&self) -> bool {
        self.depth() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_queue_send_recv() {
        let queue = AdaptiveQueue::new(10);
        let path: Arc<Path> = Arc::from(Path::new("/tmp/test.txt"));

        queue.send(FileJob::File(path.clone())).unwrap();

        match queue.recv().unwrap() {
            FileJob::File(p) => assert_eq!(p, path),
            _ => panic!("Wrong job type"),
        }
    }

    #[test]
    fn test_queue_depth() {
        let queue = AdaptiveQueue::new(10);
        assert_eq!(queue.depth(), 0);

        let path1: Arc<Path> = Arc::from(Path::new("/tmp/1"));
        let path2: Arc<Path> = Arc::from(Path::new("/tmp/2"));
        let path3: Arc<Path> = Arc::from(Path::new("/tmp/3"));

        queue.send(FileJob::File(path1)).unwrap();
        queue.send(FileJob::File(path2)).unwrap();
        queue.send(FileJob::File(path3)).unwrap();

        assert_eq!(queue.depth(), 3);

        queue.recv().unwrap();
        assert_eq!(queue.depth(), 2);

        queue.recv().unwrap();
        queue.recv().unwrap();
        assert_eq!(queue.depth(), 0);
    }

    #[test]
    fn test_empty_dir_variant() {
        let queue = AdaptiveQueue::new(5);
        let path: Arc<Path> = Arc::from(Path::new("/tmp/dir"));

        queue.send(FileJob::EmptyDir(path.clone())).unwrap();

        match queue.recv().unwrap() {
            FileJob::EmptyDir(p) => assert_eq!(p, path),
            _ => panic!("Wrong job type"),
        }
    }
}
