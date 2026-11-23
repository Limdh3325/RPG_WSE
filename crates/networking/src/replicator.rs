use crate::aoi::AoiFilter;
use crate::session::{SessionError, SharedSession};
use shared_protocol::{EntityStateWire, ServerMessage};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use tracing::{debug, warn};
use uuid::Uuid;

pub struct RegionReplicator {
    pub region_id: Uuid,
    pub sessions: Arc<RwLock<HashMap<Uuid, SharedSession>>>,
    /// Tracks which entity IDs are currently visible to each session
    session_visible: Arc<RwLock<HashMap<Uuid, HashSet<Uuid>>>>,
    /// Tracks previous entity states to detect movements and property changes
    last_known_states: Arc<RwLock<HashMap<Uuid, EntityStateWire>>>,
    pub world_revision: AtomicU64,
}

impl RegionReplicator {
    pub fn new(region_id: Uuid) -> Self {
        Self {
            region_id,
            sessions: Arc::new(RwLock::new(HashMap::new())),
            session_visible: Arc::new(RwLock::new(HashMap::new())),
            last_known_states: Arc::new(RwLock::new(HashMap::new())),
            world_revision: AtomicU64::new(1),
        }
    }

    pub fn add_session(&self, session: SharedSession) {
        let mut sessions = self
            .sessions
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        sessions.insert(session.session_id, session);
    }

    pub fn remove_session(&self, session_id: &Uuid) {
        let mut sessions = self
            .sessions
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        sessions.remove(session_id);
        let mut visible = self
            .session_visible
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        visible.remove(session_id);
    }

    pub fn session_count(&self) -> usize {
        let sessions = self
            .sessions
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        sessions.len()
    }

    pub fn get_last_known_state(&self, entity_id: &Uuid) -> Option<EntityStateWire> {
        let cache = self
            .last_known_states
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        cache.get(entity_id).cloned()
    }

    /// Sends a full baseline snapshot to a specific session, filtered by its AOI.
    pub fn send_initial_snapshot(
        &self,
        session_id: &Uuid,
        region_tick: u64,
        all_entities: &[EntityStateWire],
    ) -> Result<(), SessionError> {
        let session = {
            let sessions = self
                .sessions
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            sessions.get(session_id).cloned()
        };

        let session = match session {
            Some(s) => s,
            None => return Err(SessionError::Closed),
        };

        // Determine viewer position (use session actor position if present, or (0,0))
        let (vx, vy) = all_entities
            .iter()
            .find(|e| e.id == session.actor_id)
            .map(|e| (e.x, e.y))
            .unwrap_or((0, 0));

        let filtered_entities = AoiFilter::filter_entities(vx, vy, all_entities);
        let visible_ids: HashSet<Uuid> = filtered_entities.iter().map(|e| e.id).collect();

        {
            let mut vis = self
                .session_visible
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            vis.insert(*session_id, visible_ids);
        }

        {
            let mut state_cache = self
                .last_known_states
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            for entity in all_entities {
                state_cache.insert(entity.id, entity.clone());
            }
        }

        let revision = self.world_revision.load(Ordering::SeqCst);
        let snapshot_id = Uuid::new_v4();

        let msg = ServerMessage::WorldSnapshot {
            snapshot_id,
            world_revision: revision,
            region_tick,
            entities: filtered_entities,
        };

        session.send_nonblocking(msg)?;
        session.update_revision(revision);
        Ok(())
    }

    /// Replicates state changes to all connected sessions for this tick.
    /// Uses AOI filtering, tracks enter/leave transitions, and isolates slow clients.
    pub fn broadcast_tick(&self, region_tick: u64, all_entities: &[EntityStateWire]) -> Vec<Uuid> {
        let revision = self.world_revision.fetch_add(1, Ordering::SeqCst) + 1;
        let sessions: Vec<SharedSession> = {
            let s = self
                .sessions
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            s.values().cloned().collect()
        };

        // Current entities mapped by ID
        let current_map: HashMap<Uuid, &EntityStateWire> =
            all_entities.iter().map(|e| (e.id, e)).collect();

        // Detect changed entities compared to last known states
        let mut changed_entities: HashMap<Uuid, &EntityStateWire> = HashMap::new();
        {
            let mut state_cache = self
                .last_known_states
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());

            for entity in all_entities {
                if let Some(old_state) = state_cache.get(&entity.id) {
                    if old_state != entity {
                        changed_entities.insert(entity.id, entity);
                        state_cache.insert(entity.id, entity.clone());
                    }
                } else {
                    changed_entities.insert(entity.id, entity);
                    state_cache.insert(entity.id, entity.clone());
                }
            }

            // Remove entities that no longer exist
            state_cache.retain(|id, _| current_map.contains_key(id));
        }

        let mut disconnected_sessions = Vec::new();

        for session in sessions {
            // Find viewer position
            let (vx, vy) = current_map
                .get(&session.actor_id)
                .map(|e| (e.x, e.y))
                .unwrap_or((0, 0));

            // Compute currently in-interest entity IDs for this session
            let currently_in_aoi: HashSet<Uuid> = all_entities
                .iter()
                .filter(|e| AoiFilter::is_in_interest(vx, vy, e.x, e.y))
                .map(|e| e.id)
                .collect();

            let (entered, left) = {
                let mut vis_lock = self
                    .session_visible
                    .write()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let previously_visible = vis_lock.entry(session.session_id).or_default();

                let entered: Vec<EntityStateWire> = currently_in_aoi
                    .iter()
                    .filter(|id| !previously_visible.contains(id))
                    .filter_map(|id| current_map.get(id).copied().cloned())
                    .collect();

                let left: Vec<Uuid> = previously_visible
                    .iter()
                    .filter(|id| !currently_in_aoi.contains(id))
                    .copied()
                    .collect();

                *previously_visible = currently_in_aoi.clone();
                (entered, left)
            };

            // Entity enters
            for ent in entered {
                if let Err(err) =
                    session.send_nonblocking(ServerMessage::EntityEnter { entity: ent })
                {
                    warn!(session_id = %session.session_id, ?err, "Failed to send EntityEnter");
                    if matches!(err, SessionError::QueueFull | SessionError::Closed) {
                        disconnected_sessions.push(session.session_id);
                        break;
                    }
                }
            }

            // Entity leaves
            for entity_id in left {
                if let Err(err) = session.send_nonblocking(ServerMessage::EntityLeave { entity_id })
                {
                    warn!(session_id = %session.session_id, ?err, "Failed to send EntityLeave");
                    if matches!(err, SessionError::QueueFull | SessionError::Closed) {
                        disconnected_sessions.push(session.session_id);
                        break;
                    }
                }
            }

            // Delta changes for entities already in AOI that moved or updated
            let session_changes: Vec<EntityStateWire> = changed_entities
                .iter()
                .filter(|(id, _)| currently_in_aoi.contains(id))
                .map(|(_, e)| (*e).clone())
                .collect();

            if !session_changes.is_empty() {
                let delta_seq = session.next_delta_seq();
                let delta_msg = ServerMessage::StateDelta {
                    baseline_id: session.session_id,
                    delta_seq,
                    region_tick,
                    changed: session_changes,
                    removed: Vec::new(),
                };

                if let Err(err) = session.send_nonblocking(delta_msg) {
                    warn!(session_id = %session.session_id, ?err, "Slow client or closed session dropping delta");
                    if matches!(err, SessionError::QueueFull | SessionError::Closed) {
                        disconnected_sessions.push(session.session_id);
                    }
                } else {
                    session.update_revision(revision);
                }
            }
        }

        // Clean up disconnected / slow clients
        for sid in &disconnected_sessions {
            self.remove_session(sid);
        }

        disconnected_sessions
    }

    /// Handles client request for resynchronization after gap detection.
    pub fn handle_resync(
        &self,
        session_id: &Uuid,
        region_tick: u64,
        all_entities: &[EntityStateWire],
    ) -> Result<(), SessionError> {
        debug!(%session_id, "Handling client resync request");
        self.send_initial_snapshot(session_id, region_tick, all_entities)
    }
}
