use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use uuid::Uuid;

pub const DEFAULT_TICKET_TTL: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketData {
    pub account_id: Uuid,
    pub actor_id: Uuid,
    pub region_id: Uuid,
    pub issued_at: Instant,
    pub expires_at: Instant,
}

#[derive(Clone, Default)]
pub struct TicketStore {
    inner: Arc<Mutex<HashMap<String, TicketData>>>,
}

impl TicketStore {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Issues a single-use ticket valid for `ttl`.
    pub fn issue_ticket(
        &self,
        account_id: Uuid,
        actor_id: Uuid,
        region_id: Uuid,
        ttl: Duration,
    ) -> (String, u64) {
        let ticket = format!("tkt_{}", Uuid::new_v4().simple());
        let now = Instant::now();
        let expires_at = now + ttl;
        let data = TicketData {
            account_id,
            actor_id,
            region_id,
            issued_at: now,
            expires_at,
        };

        let mut lock = self.inner.lock().expect("ticket store poisoned");
        // Prune expired tickets opportunistically
        lock.retain(|_, v| v.expires_at > now);
        lock.insert(ticket.clone(), data);

        (ticket, ttl.as_secs())
    }

    /// Consumes a ticket once. Returns Some(TicketData) if valid and not expired,
    /// and immediately removes the ticket so it cannot be reused.
    pub fn consume_ticket(&self, ticket: &str) -> Option<TicketData> {
        let mut lock = self.inner.lock().expect("ticket store poisoned");
        match lock.remove(ticket) {
            Some(data) if data.expires_at > Instant::now() => Some(data),
            _ => None,
        }
    }

    /// Count active unconsumed tickets
    pub fn active_ticket_count(&self) -> usize {
        let now = Instant::now();
        let lock = self.inner.lock().expect("ticket store poisoned");
        lock.values().filter(|v| v.expires_at > now).count()
    }
}
