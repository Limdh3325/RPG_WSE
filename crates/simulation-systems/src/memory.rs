//! NPC Memory and Relationship management.
//!
//! Enforces memory deduplication via `source_event_id` (TEST-AI-006),
//! TTL decay, and social relationship metrics (trust, affinity, fear).

use ecs_components::{MemoryRef, NpcMemoryEntry, NpcRelationshipEntry};
use game_domain::EntityId;
use uuid::Uuid;

/// Helper manager for NPC memory operations.
#[derive(Debug, Default, Clone, Copy)]
pub struct MemoryManager;

impl MemoryManager {
    /// Records a memory into the NPC's memory repository.
    ///
    /// If `source_event_id` is provided and already exists in memory,
    /// the duplicate is strictly ignored (returns `false`).
    pub fn record_event(
        memory_ref: &mut MemoryRef,
        source_event_id: Option<Uuid>,
        kind: impl Into<String>,
        target_id: Option<EntityId>,
        severity: i32,
        happened_tick: u64,
        ttl_ticks: u64,
    ) -> bool {
        let entry = NpcMemoryEntry {
            source_event_id,
            kind: kind.into(),
            target_id,
            severity,
            happened_tick,
            ttl_ticks,
        };
        memory_ref.record_memory(entry)
    }

    /// Updates or inserts a relationship record for a target entity.
    pub fn update_relationship(
        memory_ref: &mut MemoryRef,
        target_id: EntityId,
        delta_trust: i32,
        delta_affinity: i32,
        delta_fear: i32,
    ) {
        if let Some(rel) = memory_ref
            .relationships
            .iter_mut()
            .find(|r| r.target_id == target_id)
        {
            rel.trust = (rel.trust + delta_trust).clamp(-1000, 1000);
            rel.affinity = (rel.affinity + delta_affinity).clamp(-1000, 1000);
            rel.fear = (rel.fear + delta_fear).clamp(-1000, 1000);
        } else {
            memory_ref.relationships.push(NpcRelationshipEntry {
                target_id,
                trust: delta_trust.clamp(-1000, 1000),
                affinity: delta_affinity.clamp(-1000, 1000),
                fear: delta_fear.clamp(-1000, 1000),
            });
        }
    }

    /// Prunes expired memories whose TTL has elapsed.
    pub fn prune_expired(memory_ref: &mut MemoryRef, current_tick: u64) {
        memory_ref.prune_expired(current_tick);
    }
}
