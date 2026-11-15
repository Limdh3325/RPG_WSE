//! Bi-directional index mapping persistent StableId (UUID) to Bevy ECS Entity handles.

use bevy_ecs::entity::Entity;
use bevy_ecs::prelude::Resource;
use game_domain::{DomainError, EntityId};
use std::collections::HashMap;

/// Bi-directional index between domain `EntityId` (UUID) and runtime `Entity` handles.
#[derive(Resource, Default, Debug, Clone)]
pub struct StableEntityIndex {
    by_id: HashMap<EntityId, Entity>,
    by_entity: HashMap<Entity, EntityId>,
}

impl StableEntityIndex {
    #[must_use]
    pub fn new() -> Self {
        Self {
            by_id: HashMap::new(),
            by_entity: HashMap::new(),
        }
    }

    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            by_id: HashMap::with_capacity(capacity),
            by_entity: HashMap::with_capacity(capacity),
        }
    }

    /// Inserts a new mapping between `EntityId` and `Entity`.
    ///
    /// # Errors
    /// Returns `DomainError::DuplicateEntityId` if the `EntityId` is already registered,
    /// or `DomainError::DuplicateEntityHandle` if the Bevy `Entity` is already registered.
    pub fn insert(&mut self, id: EntityId, entity: Entity) -> Result<(), DomainError> {
        if self.by_id.contains_key(&id) {
            return Err(DomainError::DuplicateEntityId(id));
        }
        if let Some(existing_id) = self.by_entity.get(&entity) {
            return Err(DomainError::DuplicateEntityHandle(format!(
                "{entity:?} already bound to stable id {existing_id}"
            )));
        }
        self.by_id.insert(id, entity);
        self.by_entity.insert(entity, id);
        Ok(())
    }

    /// Removes an entity by its stable `EntityId`. Returns the associated `Entity` if found.
    pub fn remove_by_id(&mut self, id: EntityId) -> Option<Entity> {
        if let Some(entity) = self.by_id.remove(&id) {
            self.by_entity.remove(&entity);
            Some(entity)
        } else {
            None
        }
    }

    /// Removes an entity by its Bevy `Entity`. Returns the associated `EntityId` if found.
    pub fn remove_by_entity(&mut self, entity: Entity) -> Option<EntityId> {
        if let Some(id) = self.by_entity.remove(&entity) {
            self.by_id.remove(&id);
            Some(id)
        } else {
            None
        }
    }

    #[must_use]
    pub fn get_entity(&self, id: EntityId) -> Option<Entity> {
        self.by_id.get(&id).copied()
    }

    #[must_use]
    pub fn get_stable_id(&self, entity: Entity) -> Option<EntityId> {
        self.by_entity.get(&entity).copied()
    }

    #[must_use]
    pub fn contains_id(&self, id: EntityId) -> bool {
        self.by_id.contains_key(&id)
    }

    #[must_use]
    pub fn contains_entity(&self, entity: Entity) -> bool {
        self.by_entity.contains_key(&entity)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    pub fn clear(&mut self) {
        self.by_id.clear();
        self.by_entity.clear();
    }

    pub fn iter(&self) -> impl Iterator<Item = (&EntityId, &Entity)> {
        self.by_id.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::world::World;

    #[test]
    fn test_index_crud_and_rejection() {
        let mut index = StableEntityIndex::new();
        let id1 = EntityId::from_u128(101);
        let id2 = EntityId::from_u128(102);

        let mut world = World::new();
        let entity1 = world.spawn_empty().id();
        let entity2 = world.spawn_empty().id();

        assert!(index.insert(id1, entity1).is_ok());
        assert_eq!(index.len(), 1);
        assert_eq!(index.get_entity(id1), Some(entity1));
        assert_eq!(index.get_stable_id(entity1), Some(id1));

        // Duplicate rejection
        let err = index.insert(id1, entity2).unwrap_err();
        assert_eq!(err, DomainError::DuplicateEntityId(id1));

        // Insert second
        assert!(index.insert(id2, entity2).is_ok());
        assert_eq!(index.len(), 2);

        // Remove by id
        let removed = index.remove_by_id(id1);
        assert_eq!(removed, Some(entity1));
        assert_eq!(index.len(), 1);
        assert_eq!(index.get_entity(id1), None);
        assert_eq!(index.get_stable_id(entity1), None);

        // Remove by entity
        let removed_id = index.remove_by_entity(entity2);
        assert_eq!(removed_id, Some(id2));
        assert!(index.is_empty());
    }

    #[test]
    fn test_index_rejects_duplicate_runtime_entity() {
        let mut index = StableEntityIndex::new();
        let id1 = EntityId::from_u128(201);
        let id2 = EntityId::from_u128(202);

        let mut world = World::new();
        let entity = world.spawn_empty().id();

        assert!(index.insert(id1, entity).is_ok());
        let err = index.insert(id2, entity).unwrap_err();
        assert!(matches!(err, DomainError::DuplicateEntityHandle(_)));
        assert_eq!(index.get_entity(id1), Some(entity));
        assert_eq!(index.get_stable_id(entity), Some(id1));
        assert_eq!(index.len(), 1);
    }
}
