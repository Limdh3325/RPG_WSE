//! Region-level ECS World container with StableId indexing and canonical DTO projection.

use crate::components::{
    Collider, FactionMembership, Health, InventoryProjection, Needs, Personality, Position,
    StableId, Velocity,
};
use crate::grid::MapGrid;
use crate::index::StableEntityIndex;
use bevy_ecs::entity::Entity;
use bevy_ecs::prelude::Resource;
use bevy_ecs::world::World;
use game_domain::{
    DomainError, EntityId, EntityStateDto, RegionHealth, RegionId, RegionStateDto, RegionStatus,
};

/// High-level runtime metadata stored as a Bevy ECS resource.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegionMetaResource {
    pub region_id: RegionId,
    pub epoch: u64,
    pub tick: u64,
    pub status: RegionStatus,
    pub health: RegionHealth,
}

/// Self-contained ECS simulation world for a single Region.
pub struct RegionWorld {
    pub world: World,
    pub index: StableEntityIndex,
    pub grid: MapGrid,
    pub meta: RegionMetaResource,
}

impl RegionWorld {
    /// Creates a new `RegionWorld` with a custom grid dimension.
    #[must_use]
    pub fn new(region_id: RegionId, epoch: u64, width_tiles: u32, height_tiles: u32) -> Self {
        let mut world = World::new();
        let grid = MapGrid::new(width_tiles, height_tiles);
        let index = StableEntityIndex::new();
        let meta = RegionMetaResource {
            region_id,
            epoch,
            tick: 0,
            status: RegionStatus::Loading,
            health: RegionHealth::Healthy,
        };

        world.insert_resource(meta);
        world.insert_resource(grid.clone());
        world.insert_resource(index.clone());

        Self {
            world,
            index,
            grid,
            meta,
        }
    }

    /// Creates a standard 64x64 settlement region world.
    #[must_use]
    pub fn new_default_64x64(region_id: RegionId, epoch: u64) -> Self {
        let mut world = World::new();
        let grid = MapGrid::default_64x64();
        let index = StableEntityIndex::new();
        let meta = RegionMetaResource {
            region_id,
            epoch,
            tick: 0,
            status: RegionStatus::Loading,
            health: RegionHealth::Healthy,
        };

        world.insert_resource(meta);
        world.insert_resource(grid.clone());
        world.insert_resource(index.clone());

        Self {
            world,
            index,
            grid,
            meta,
        }
    }

    /// Spawns an entity from a canonical `EntityStateDto`.
    ///
    /// # Errors
    /// Returns `DomainError::DuplicateEntityId` if the ID is already present.
    /// Returns `DomainError::OutOfBoundsPosition` if coordinates exceed the map grid.
    pub fn spawn_entity(&mut self, state: EntityStateDto) -> Result<Entity, DomainError> {
        if self.index.contains_id(state.id) {
            return Err(DomainError::DuplicateEntityId(state.id));
        }

        if !self
            .grid
            .is_in_bounds_millitile(state.position.x_millitile, state.position.y_millitile)
        {
            return Err(DomainError::OutOfBoundsPosition {
                x: state.position.x_millitile,
                y: state.position.y_millitile,
                min_x: 0,
                max_x: self.grid.max_millitile_x() - 1,
                min_y: 0,
                max_y: self.grid.max_millitile_y() - 1,
            });
        }

        let health_comp = state.health.map(Health::from_dto).transpose()?;

        let mut entity_commands = self
            .world
            .spawn((StableId::new(state.id), Position::from_dto(state.position)));

        if let Some(vel) = state.velocity {
            entity_commands.insert(Velocity::from_dto(vel));
        }
        if let Some(health) = health_comp {
            entity_commands.insert(health);
        }
        if let Some(needs) = state.needs {
            entity_commands.insert(Needs::from_dto(needs));
        }
        if let Some(personality) = state.personality {
            entity_commands.insert(Personality::from_dto(personality));
        }
        if let Some(inv) = state.inventory {
            entity_commands.insert(InventoryProjection::from_dto(inv));
        }
        if let Some(faction) = state.faction {
            entity_commands.insert(FactionMembership::from_dto(faction));
        }
        if let Some(collider) = state.collider {
            entity_commands.insert(Collider::from_dto(collider));
        }

        let entity = entity_commands.id();
        if let Err(err) = self.index.insert(state.id, entity) {
            self.world.despawn(entity);
            return Err(err);
        }
        self.sync_index_resource();
        Ok(entity)
    }

    /// Despawns an entity by its stable `EntityId`.
    ///
    /// # Errors
    /// Returns `DomainError::EntityNotFound` if no entity matches the given `EntityId`.
    pub fn despawn_entity(&mut self, id: EntityId) -> Result<(), DomainError> {
        let entity = self
            .index
            .remove_by_id(id)
            .ok_or(DomainError::EntityNotFound(id))?;

        if !self.world.despawn(entity) {
            return Err(DomainError::EcsError(format!(
                "failed to despawn entity {entity:?} from World"
            )));
        }

        self.sync_index_resource();
        Ok(())
    }

    #[must_use]
    pub fn get_entity(&self, id: EntityId) -> Option<Entity> {
        self.index.get_entity(id)
    }

    #[must_use]
    pub fn get_stable_id(&self, entity: Entity) -> Option<EntityId> {
        self.index.get_stable_id(entity)
    }

    /// Reads the position of an entity by its stable `EntityId`.
    ///
    /// # Errors
    /// Returns `DomainError::EntityNotFound` or `DomainError::ComponentMissing`.
    pub fn get_position(&self, id: EntityId) -> Result<Position, DomainError> {
        let entity = self.get_entity(id).ok_or(DomainError::EntityNotFound(id))?;
        self.world
            .get::<Position>(entity)
            .copied()
            .ok_or(DomainError::ComponentMissing("Position"))
    }

    /// Reads the health of an entity by its stable `EntityId`.
    pub fn get_health(&self, id: EntityId) -> Result<Health, DomainError> {
        let entity = self.get_entity(id).ok_or(DomainError::EntityNotFound(id))?;
        self.world
            .get::<Health>(entity)
            .copied()
            .ok_or(DomainError::ComponentMissing("Health"))
    }

    /// Reads the needs of an entity by its stable `EntityId`.
    pub fn get_needs(&self, id: EntityId) -> Result<Needs, DomainError> {
        let entity = self.get_entity(id).ok_or(DomainError::EntityNotFound(id))?;
        self.world
            .get::<Needs>(entity)
            .copied()
            .ok_or(DomainError::ComponentMissing("Needs"))
    }

    #[must_use]
    pub fn entity_count(&self) -> usize {
        self.index.len()
    }

    /// Exports full canonical snapshot of the region and entities, sorted deterministically by `EntityId`.
    #[must_use]
    pub fn export_canonical(&mut self) -> RegionStateDto {
        let mut query = self.world.query::<(
            &StableId,
            &Position,
            Option<&Velocity>,
            Option<&Health>,
            Option<&Needs>,
            Option<&Personality>,
            Option<&InventoryProjection>,
            Option<&FactionMembership>,
            Option<&Collider>,
        )>();

        let mut entities = Vec::with_capacity(self.index.len());
        for (stable_id, pos, vel, health, needs, personality, inv, faction, collider) in
            query.iter(&self.world)
        {
            entities.push(EntityStateDto {
                id: stable_id.id(),
                position: pos.to_dto(),
                velocity: vel.map(Velocity::to_dto),
                health: health.map(Health::to_dto),
                needs: needs.map(Needs::to_dto),
                personality: personality.map(Personality::to_dto),
                inventory: inv.map(InventoryProjection::to_dto),
                faction: faction.map(FactionMembership::to_dto),
                collider: collider.map(Collider::to_dto),
            });
        }

        // Canonical deterministic sort by stable UUID
        entities.sort_by_key(|e| e.id);

        RegionStateDto {
            region_id: self.meta.region_id,
            epoch: self.meta.epoch,
            tick: self.meta.tick,
            status: self.meta.status,
            entities,
        }
    }

    /// Imports a canonical snapshot into this region world, replacing current state atomically.
    ///
    /// # Errors
    /// Returns error if any entity in the snapshot violates boundaries or has duplicate IDs.
    /// If an error occurs, the existing world state remains completely unmodified.
    pub fn import_canonical(&mut self, snapshot: RegionStateDto) -> Result<(), DomainError> {
        let mut staged = Self::new(
            snapshot.region_id,
            snapshot.epoch,
            self.grid.width_tiles(),
            self.grid.height_tiles(),
        );
        staged.meta.tick = snapshot.tick;
        staged.meta.status = snapshot.status;

        for entity_dto in snapshot.entities {
            staged.spawn_entity(entity_dto)?;
        }

        staged.sync_meta_resource();
        staged.sync_index_resource();

        *self = staged;
        Ok(())
    }

    fn sync_index_resource(&mut self) {
        if let Some(mut res) = self.world.get_resource_mut::<StableEntityIndex>() {
            *res = self.index.clone();
        }
    }

    fn sync_meta_resource(&mut self) {
        if let Some(mut res) = self.world.get_resource_mut::<RegionMetaResource>() {
            *res = self.meta;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use game_domain::{CombatStatus, PositionDto};

    #[test]
    fn test_spawn_query_and_despawn() {
        let region_id = RegionId::from_u128(1);
        let mut region = RegionWorld::new_default_64x64(region_id, 1);

        let entity_id = EntityId::from_u128(42);
        let dto = EntityStateDto {
            id: entity_id,
            position: PositionDto {
                x_millitile: 5000,
                y_millitile: 5000,
            },
            velocity: None,
            health: Some(game_domain::HealthDto {
                current: 100,
                max: 100,
                status: CombatStatus::Alive,
            }),
            needs: None,
            personality: None,
            inventory: None,
            faction: None,
            collider: None,
        };

        let entity = region.spawn_entity(dto).unwrap();
        assert_eq!(region.entity_count(), 1);
        assert_eq!(region.get_entity(entity_id), Some(entity));
        assert_eq!(region.get_stable_id(entity), Some(entity_id));

        let pos = region.get_position(entity_id).unwrap();
        assert_eq!(pos.x_millitile, 5000);
        assert_eq!(pos.y_millitile, 5000);

        let health = region.get_health(entity_id).unwrap();
        assert_eq!(health.current, 100);

        // Despawn
        region.despawn_entity(entity_id).unwrap();
        assert_eq!(region.entity_count(), 0);
        assert_eq!(region.get_entity(entity_id), None);
    }

    #[test]
    fn test_duplicate_entity_rejected() {
        let region_id = RegionId::from_u128(1);
        let mut region = RegionWorld::new_default_64x64(region_id, 1);

        let id = EntityId::from_u128(100);
        let dto1 = EntityStateDto {
            id,
            position: PositionDto {
                x_millitile: 1000,
                y_millitile: 1000,
            },
            velocity: None,
            health: None,
            needs: None,
            personality: None,
            inventory: None,
            faction: None,
            collider: None,
        };
        let dto2 = dto1.clone();

        assert!(region.spawn_entity(dto1).is_ok());
        let err = region.spawn_entity(dto2).unwrap_err();
        assert_eq!(err, DomainError::DuplicateEntityId(id));
    }

    #[test]
    fn test_out_of_bounds_rejected() {
        let region_id = RegionId::from_u128(1);
        let mut region = RegionWorld::new_default_64x64(region_id, 1);

        let id = EntityId::from_u128(200);
        let dto = EntityStateDto {
            id,
            position: PositionDto {
                x_millitile: 70_000, // max is 63_999
                y_millitile: 1000,
            },
            velocity: None,
            health: None,
            needs: None,
            personality: None,
            inventory: None,
            faction: None,
            collider: None,
        };

        let err = region.spawn_entity(dto).unwrap_err();
        assert!(matches!(err, DomainError::OutOfBoundsPosition { .. }));
    }
}
