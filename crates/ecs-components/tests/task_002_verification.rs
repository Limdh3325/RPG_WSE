//! Verification test suite for TASK-002: Region and Entity Model.
//!
//! Validates:
//! - 64x64 test settlement region setup
//! - Complete component registry (Position, Velocity, Health, Needs, Inventory, etc.)
//! - StableId (UUID) <-> Bevy Entity bidirectional mapping
//! - Duplicate ID rejection
//! - Canonical state DTO export and restoration
//! - 10,000 Entity lifecycle stress test (create, lookup, despawn, reload)

use ecs_components::{DEFAULT_GRID_HEIGHT_TILES, DEFAULT_GRID_WIDTH_TILES, RegionWorld};
use game_domain::{
    ColliderDto, CombatStatus, DomainError, EntityId, EntityStateDto, FactionMembershipDto,
    HealthDto, InventoryProjectionDto, InventorySlotDto, NeedsDto, PersonalityDto, PositionDto,
    RegionId, VelocityDto,
};
use uuid::Uuid;

#[test]
fn test_64x64_test_region_world_setup() {
    let region_id = RegionId::from_u128(1001);
    let region = RegionWorld::new_default_64x64(region_id, 1);

    assert_eq!(region.grid.width_tiles(), DEFAULT_GRID_WIDTH_TILES);
    assert_eq!(region.grid.height_tiles(), DEFAULT_GRID_HEIGHT_TILES);
    assert_eq!(region.grid.width_tiles(), 64);
    assert_eq!(region.grid.height_tiles(), 64);

    // Millitile boundaries: 0 <= x < 64_000, 0 <= y < 64_000
    assert!(region.grid.is_in_bounds_millitile(0, 0));
    assert!(region.grid.is_in_bounds_millitile(63_999, 63_999));
    assert!(!region.grid.is_in_bounds_millitile(64_000, 1000));
    assert!(!region.grid.is_in_bounds_millitile(-1, 1000));
}

#[test]
fn test_entity_lifecycle_and_component_registry() {
    let region_id = RegionId::from_u128(2001);
    let mut region = RegionWorld::new_default_64x64(region_id, 1);

    let entity_id = EntityId::from_u128(999);
    let faction_uuid = Uuid::from_u128(77);

    let state = EntityStateDto {
        id: entity_id,
        position: PositionDto {
            x_millitile: 12_500,
            y_millitile: 34_500,
        },
        velocity: Some(VelocityDto { dx: 150, dy: -200 }),
        health: Some(HealthDto {
            current: 80,
            max: 100,
            status: CombatStatus::Alive,
        }),
        needs: Some(NeedsDto {
            hunger: 250,
            fatigue: 100,
            safety: 900,
        }),
        personality: Some(PersonalityDto {
            courage: 500,
            greed: -200,
            loyalty: 800,
        }),
        inventory: Some(InventoryProjectionDto {
            committed_version: 1,
            slots: vec![
                InventorySlotDto {
                    slot_index: 0,
                    item_id: "iron_sword".into(),
                    quantity: 1,
                },
                InventorySlotDto {
                    slot_index: 1,
                    item_id: "bread".into(),
                    quantity: 5,
                },
            ],
        }),
        faction: Some(FactionMembershipDto {
            faction_id: faction_uuid,
            standing: 50,
        }),
        collider: Some(ColliderDto {
            width_millitile: 800,
            height_millitile: 800,
            is_trigger: false,
        }),
    };

    let entity = region.spawn_entity(state).expect("spawn should succeed");
    assert_eq!(region.entity_count(), 1);

    // Verify all components exist in ECS World
    let pos = region.get_position(entity_id).expect("position exists");
    assert_eq!(pos.x_millitile, 12_500);
    assert_eq!(pos.y_millitile, 34_500);
    assert_eq!(pos.tile_x(), 12);
    assert_eq!(pos.tile_y(), 34);

    let mut health = region.get_health(entity_id).expect("health exists");
    assert_eq!(health.current, 80);
    assert_eq!(health.max, 100);
    let status = health.apply_damage(30);
    assert_eq!(status, CombatStatus::Alive);
    assert_eq!(health.current, 50);

    let status = health.apply_damage(60);
    assert_eq!(status, CombatStatus::Dead);
    assert_eq!(health.current, 0);

    let needs = region.get_needs(entity_id).expect("needs exists");
    assert_eq!(needs.hunger, 250);
    assert_eq!(needs.fatigue, 100);
    assert_eq!(needs.safety, 900);

    // Despawn
    region
        .despawn_entity(entity_id)
        .expect("despawn should succeed");
    assert_eq!(region.entity_count(), 0);
    assert_eq!(region.get_entity(entity_id), None);
    assert_eq!(region.get_stable_id(entity), None);
}

#[test]
fn test_stable_id_mapping_and_duplicate_rejection() {
    let region_id = RegionId::from_u128(3001);
    let mut region = RegionWorld::new_default_64x64(region_id, 1);

    let id1 = EntityId::from_u128(111);
    let id2 = EntityId::from_u128(222);

    let e1 = region
        .spawn_entity(EntityStateDto {
            id: id1,
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
        })
        .unwrap();

    let e2 = region
        .spawn_entity(EntityStateDto {
            id: id2,
            position: PositionDto {
                x_millitile: 2000,
                y_millitile: 2000,
            },
            velocity: None,
            health: None,
            needs: None,
            personality: None,
            inventory: None,
            faction: None,
            collider: None,
        })
        .unwrap();

    assert_eq!(region.get_entity(id1), Some(e1));
    assert_eq!(region.get_entity(id2), Some(e2));
    assert_eq!(region.get_stable_id(e1), Some(id1));
    assert_eq!(region.get_stable_id(e2), Some(id2));

    // Attempt duplicate registration of id1
    let dup_res = region.spawn_entity(EntityStateDto {
        id: id1,
        position: PositionDto {
            x_millitile: 3000,
            y_millitile: 3000,
        },
        velocity: None,
        health: None,
        needs: None,
        personality: None,
        inventory: None,
        faction: None,
        collider: None,
    });

    assert_eq!(dup_res.unwrap_err(), DomainError::DuplicateEntityId(id1));
    assert_eq!(region.entity_count(), 2);
}

#[test]
fn test_canonical_state_dto_export_and_restore() {
    let region_id = RegionId::from_u128(4001);
    let mut region = RegionWorld::new_default_64x64(region_id, 1);

    // Spawn 10 entities
    for i in 0..10 {
        let id = EntityId::from_u128(500 + i);
        let coord = (i as i64 + 1) * 2000;
        region
            .spawn_entity(EntityStateDto {
                id,
                position: PositionDto {
                    x_millitile: coord,
                    y_millitile: coord,
                },
                velocity: Some(VelocityDto {
                    dx: i as i32 * 10,
                    dy: -(i as i32 * 10),
                }),
                health: Some(HealthDto {
                    current: 100 - (i as i32 * 5),
                    max: 100,
                    status: CombatStatus::Alive,
                }),
                needs: Some(NeedsDto {
                    hunger: i as u16 * 50,
                    fatigue: 100,
                    safety: 800,
                }),
                personality: None,
                inventory: None,
                faction: None,
                collider: None,
            })
            .expect("spawn entity");
    }

    assert_eq!(region.entity_count(), 10);

    // Export canonical snapshot
    let snapshot = region.export_canonical();
    assert_eq!(snapshot.region_id, region_id);
    assert_eq!(snapshot.entities.len(), 10);

    // Create a fresh region and restore
    let mut restored_region = RegionWorld::new_default_64x64(region_id, 1);
    assert_eq!(restored_region.entity_count(), 0);

    restored_region
        .import_canonical(snapshot)
        .expect("import snapshot");
    assert_eq!(restored_region.entity_count(), 10);

    // Verify all 10 entities exist with identical IDs and component state
    for i in 0..10 {
        let id = EntityId::from_u128(500 + i);
        let coord = (i as i64 + 1) * 2000;

        let pos = restored_region.get_position(id).expect("position exists");
        assert_eq!(pos.x_millitile, coord);
        assert_eq!(pos.y_millitile, coord);

        let health = restored_region.get_health(id).expect("health exists");
        assert_eq!(health.current, 100 - (i as i32 * 5));

        let needs = restored_region.get_needs(id).expect("needs exists");
        assert_eq!(needs.hunger, i as u16 * 50);
    }
}

#[test]
fn test_10000_entity_stress_and_index_integrity() {
    let region_id = RegionId::from_u128(5001);
    let mut region = RegionWorld::new_default_64x64(region_id, 1);

    const TOTAL_ENTITIES: usize = 10_000;

    // 1. Spawn 10,000 entities distributed across the 64x64 settlement
    for i in 0..TOTAL_ENTITIES {
        let id = EntityId::from_u128(10_000_000 + i as u128);
        // Distribute safely within 0..60_000 millitiles
        let x = (i % 60) as i64 * 1000 + 500;
        let y = ((i / 60) % 60) as i64 * 1000 + 500;

        let res = region.spawn_entity(EntityStateDto {
            id,
            position: PositionDto {
                x_millitile: x,
                y_millitile: y,
            },
            velocity: Some(VelocityDto { dx: 10, dy: 10 }),
            health: Some(HealthDto {
                current: 100,
                max: 100,
                status: CombatStatus::Alive,
            }),
            needs: Some(NeedsDto {
                hunger: (i % 1000) as u16,
                fatigue: (i % 500) as u16,
                safety: 500,
            }),
            personality: None,
            inventory: None,
            faction: None,
            collider: None,
        });

        assert!(res.is_ok(), "failed to spawn entity {i}: {:?}", res.err());
    }

    assert_eq!(region.entity_count(), TOTAL_ENTITIES);

    // 2. Verify all 10,000 entities can be resolved through index
    for i in 0..TOTAL_ENTITIES {
        let id = EntityId::from_u128(10_000_000 + i as u128);
        let entity = region.get_entity(id).expect("entity must be indexed");
        let stable_id = region.get_stable_id(entity).expect("must map to stable id");
        assert_eq!(stable_id, id);
    }

    // 3. Verify duplicate ID rejection among 10,000
    let dup_id = EntityId::from_u128(10_000_000 + 4242);
    let dup_err = region
        .spawn_entity(EntityStateDto {
            id: dup_id,
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
        })
        .unwrap_err();
    assert_eq!(dup_err, DomainError::DuplicateEntityId(dup_id));

    // 4. Despawn 5,000 entities (even numbered indices)
    for i in (0..TOTAL_ENTITIES).step_by(2) {
        let id = EntityId::from_u128(10_000_000 + i as u128);
        region
            .despawn_entity(id)
            .unwrap_or_else(|e| panic!("failed to despawn {id}: {e}"));
    }

    assert_eq!(region.entity_count(), 5000);

    // Verify index integrity: even are gone, odd remain
    for i in 0..TOTAL_ENTITIES {
        let id = EntityId::from_u128(10_000_000 + i as u128);
        if i % 2 == 0 {
            assert!(region.get_entity(id).is_none());
        } else {
            assert!(region.get_entity(id).is_some());
        }
    }

    // 5. Re-spawn 2,500 new entities
    for i in 0..2500 {
        let id = EntityId::from_u128(20_000_000 + i as u128);
        let x = (i % 60) as i64 * 1000 + 500;
        let y = ((i / 60) % 60) as i64 * 1000 + 500;

        region
            .spawn_entity(EntityStateDto {
                id,
                position: PositionDto {
                    x_millitile: x,
                    y_millitile: y,
                },
                velocity: None,
                health: None,
                needs: None,
                personality: None,
                inventory: None,
                faction: None,
                collider: None,
            })
            .expect("respawn entity");
    }

    assert_eq!(region.entity_count(), 7500);

    // 6. Export and restore canonical state
    let snapshot = region.export_canonical();
    assert_eq!(snapshot.entities.len(), 7500);

    let mut fresh_world = RegionWorld::new_default_64x64(region_id, 1);
    fresh_world
        .import_canonical(snapshot)
        .expect("canonical reload");
    assert_eq!(fresh_world.entity_count(), 7500);
}

#[test]
fn test_index_rejects_duplicate_runtime_entity_and_preserves_symmetry() {
    let mut world = bevy_ecs::world::World::new();
    let entity = world.spawn_empty().id();
    let mut index = ecs_components::StableEntityIndex::new();
    let first = EntityId::from_u128(1);
    let second = EntityId::from_u128(2);

    assert!(index.insert(first, entity).is_ok());
    let err = index.insert(second, entity).unwrap_err();
    assert!(
        matches!(err, DomainError::DuplicateEntityHandle(_)),
        "one ECS Entity must not map to two stable IDs"
    );
    assert_eq!(index.get_entity(first), Some(entity));
    assert_eq!(index.get_stable_id(entity), Some(first));
    assert_eq!(index.len(), 1);
}

#[test]
fn test_failed_canonical_import_is_atomic() {
    let original_id = EntityId::from_u128(1);
    let invalid_id = EntityId::from_u128(2);
    let mut region = RegionWorld::new_default_64x64(RegionId::from_u128(1), 1);

    let original_dto = EntityStateDto {
        id: original_id,
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
    region.spawn_entity(original_dto).unwrap();

    let invalid_snapshot = game_domain::RegionStateDto {
        region_id: RegionId::from_u128(2),
        epoch: 2,
        tick: 3,
        status: game_domain::RegionStatus::Active,
        entities: vec![
            EntityStateDto {
                id: invalid_id,
                position: PositionDto {
                    x_millitile: 2000,
                    y_millitile: 2000,
                },
                velocity: None,
                health: None,
                needs: None,
                personality: None,
                inventory: None,
                faction: None,
                collider: None,
            },
            EntityStateDto {
                id: invalid_id, // duplicate ID causing error
                position: PositionDto {
                    x_millitile: 3000,
                    y_millitile: 3000,
                },
                velocity: None,
                health: None,
                needs: None,
                personality: None,
                inventory: None,
                faction: None,
                collider: None,
            },
        ],
    };

    assert!(region.import_canonical(invalid_snapshot).is_err());
    assert_eq!(
        region.entity_count(),
        1,
        "a failed import must preserve prior world atomically"
    );
    assert!(region.get_entity(original_id).is_some());
}

#[test]
fn test_health_invariants_and_contradictory_status_rejected() {
    use ecs_components::Health;

    // max <= 0
    let err = Health::from_dto(HealthDto {
        current: 10,
        max: 0,
        status: CombatStatus::Alive,
    });
    assert!(matches!(err, Err(DomainError::InvalidHealth { .. })));

    // current > max
    let err = Health::from_dto(HealthDto {
        current: 150,
        max: 100,
        status: CombatStatus::Alive,
    });
    assert!(matches!(err, Err(DomainError::InvalidHealth { .. })));

    // current < 0
    let err = Health::from_dto(HealthDto {
        current: -5,
        max: 100,
        status: CombatStatus::Alive,
    });
    assert!(matches!(err, Err(DomainError::InvalidHealth { .. })));

    // contradictory status: 0 current with Alive status
    let err = Health::from_dto(HealthDto {
        current: 0,
        max: 100,
        status: CombatStatus::Alive,
    });
    assert!(matches!(err, Err(DomainError::InvalidHealth { .. })));

    // contradictory status: positive current with Dead status
    let err = Health::from_dto(HealthDto {
        current: 50,
        max: 100,
        status: CombatStatus::Dead,
    });
    assert!(matches!(err, Err(DomainError::InvalidHealth { .. })));

    // valid health
    let ok = Health::from_dto(HealthDto {
        current: 50,
        max: 100,
        status: CombatStatus::Alive,
    });
    assert!(ok.is_ok());
}
