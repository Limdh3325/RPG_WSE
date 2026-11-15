use shared_protocol::ServerMessage;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use uuid::Uuid;

pub const DEFAULT_MAX_QUEUE_SIZE: usize = 256;
pub const MAX_COMMANDS_PER_SECOND: u32 = 60;

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("Session queue full (slow client)")]
    QueueFull,
    #[error("Session closed")]
    Closed,
    #[error("Rate limit exceeded ({0} cmd/s limit)")]
    RateLimitExceeded(u32),
}

/// Simple token bucket / fixed-window rate limiter.
#[derive(Debug)]
pub struct CommandRateLimiter {
    max_per_sec: u32,
    window_start: Instant,
    count_in_window: u32,
}

impl CommandRateLimiter {
    pub fn new(max_per_sec: u32) -> Self {
        Self {
            max_per_sec,
            window_start: Instant::now(),
            count_in_window: 0,
        }
    }

    pub fn check_and_record(&mut self) -> bool {
        let now = Instant::now();
        if now.duration_since(self.window_start) >= Duration::from_secs(1) {
            self.window_start = now;
            self.count_in_window = 0;
        }

        if self.count_in_window < self.max_per_sec {
            self.count_in_window += 1;
            true
        } else {
            false
        }
    }
}

use std::sync::RwLock;

pub struct Session {
    pub session_id: Uuid,
    pub account_id: Uuid,
    pub actor_id: Uuid,
    pub region_id: RwLock<Uuid>,
    pub tx: mpsc::Sender<ServerMessage>,
    pub last_seen_revision: AtomicU64,
    pub last_sent_delta_seq: AtomicU64,
}

impl Session {
    pub fn new(
        session_id: Uuid,
        account_id: Uuid,
        actor_id: Uuid,
        region_id: Uuid,
        tx: mpsc::Sender<ServerMessage>,
    ) -> Self {
        Self {
            session_id,
            account_id,
            actor_id,
            region_id: RwLock::new(region_id),
            tx,
            last_seen_revision: AtomicU64::new(0),
            last_sent_delta_seq: AtomicU64::new(0),
        }
    }

    pub fn region_id(&self) -> Uuid {
        *self
            .region_id
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn set_region_id(&self, new_region: Uuid) {
        *self
            .region_id
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = new_region;
    }

    /// Sends a message to the client. Uses non-blocking `try_send` to ensure slow clients
    /// cannot block the simulation or region worker tick.
    pub fn send_nonblocking(&self, msg: ServerMessage) -> Result<(), SessionError> {
        self.tx.try_send(msg).map_err(|err| match err {
            mpsc::error::TrySendError::Full(_) => SessionError::QueueFull,
            mpsc::error::TrySendError::Closed(_) => SessionError::Closed,
        })
    }

    pub fn update_revision(&self, rev: u64) {
        self.last_seen_revision.store(rev, Ordering::Relaxed);
    }

    pub fn next_delta_seq(&self) -> u64 {
        self.last_sent_delta_seq.fetch_add(1, Ordering::SeqCst) + 1
    }
}

pub type SharedSession = Arc<Session>;
