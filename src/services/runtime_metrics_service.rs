use std::sync::atomic::{AtomicU32, Ordering};

static ACTIVE_CHUNK_CONNECTIONS: AtomicU32 = AtomicU32::new(0);

pub struct ActiveChunkConnectionGuard;

impl ActiveChunkConnectionGuard {
    pub fn enter() -> Self {
        ACTIVE_CHUNK_CONNECTIONS.fetch_add(1, Ordering::Relaxed);
        Self
    }
}

impl Drop for ActiveChunkConnectionGuard {
    fn drop(&mut self) {
        ACTIVE_CHUNK_CONNECTIONS.fetch_sub(1, Ordering::Relaxed);
    }
}

pub fn active_chunk_connections() -> u32 {
    ACTIVE_CHUNK_CONNECTIONS.load(Ordering::Relaxed)
}
