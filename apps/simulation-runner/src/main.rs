use clap::Parser;
use ecs_components::components::{CombatStats, Health, Needs, Personality, Position};
use ecs_components::region_world::RegionWorld;
use game_domain::id::{EntityId, RegionId, SessionId};
use simulation_core::engine::SimulationEngine;
use simulation_systems::economy::SettlementEconomy;
use simulation_systems::faction::FactionManager;
use simulation_systems::integrated_hooks::IntegratedSimulationHooks;
use simulation_systems::rule_dsl::DynamicRuleRegistry;
use simulation_systems::scheduler::NpcScheduler;
use simulation_systems::system::NpcAiSystem;

#[derive(Parser)]
#[command(
    version,
    about = "WSE headless runner CLI; executes deterministic simulation ticks in M1"
)]
struct Args {
    #[arg(long, default_value_t = 12345)]
    seed: u64,
    #[arg(long, default_value_t = 10000, value_parser = clap::value_parser!(u64).range(1..))]
    ticks: u64,
    /// Validate CLI without claiming any simulation ticks were executed.
    #[arg(long)]
    check: bool,
}

fn main() -> std::process::ExitCode {
    let args = Args::parse();
    if args.check {
        println!(
            "{}",
            serde_json::json!({
                "milestone": "M1",
                "seed": args.seed,
                "requested_ticks": args.ticks,
                "executed_ticks": 0,
                "simulation_available": false,
                "status": "configuration_valid"
            })
        );
        return std::process::ExitCode::SUCCESS;
    }

    let region_id = RegionId::from_u128(1);
    let mut world = RegionWorld::new_default_64x64(region_id, 1);

    // Bootstrap initial pioneer settlement NPCs (villager & merchant) for the integrated simulation
    let villager_id = EntityId::from_u128(101);
    let villager_entity = world.world.spawn((
        Position { x_millitile: 32_000, y_millitile: 32_000 },
        Health::new(100, 100).expect("valid health"),
        CombatStats::new(1000, 10, 2, 5),
        Personality::default(),
        Needs::new(100, 100, 1000),
    )).id();
    let _ = world.index.insert(villager_id, villager_entity);

    let mut engine = SimulationEngine::new(world, args.seed);

    let session_id = SessionId::from_u128(args.seed as u128);
    let mut hooks = IntegratedSimulationHooks::new(
        NpcAiSystem::default(),
        NpcScheduler::default(),
        SettlementEconomy::new_default(1),
        FactionManager::default(),
        DynamicRuleRegistry::default(),
        session_id,
        region_id,
    );

    match engine.run_ticks_with_hooks(args.ticks, &mut hooks) {
        Ok(summary) => {
            println!(
                "{}",
                serde_json::json!({
                    "milestone": "M1",
                    "seed": summary.seed,
                    "requested_ticks": args.ticks,
                    "executed_ticks": summary.total_ticks_executed,
                    "final_state_hash": summary.final_state_hash,
                    "total_events_emitted": summary.total_events_emitted,
                    "economy": {
                        "settlement_id": hooks.economy.settlement_id,
                        "food_stock": hooks.economy.food_stock,
                        "current_price": hooks.economy.current_price,
                        "security_index": hooks.economy.security_index,
                        "shortage_active": hooks.economy.shortage_active,
                        "ledger_conservation_valid": hooks.food_ledger.audit_conservation(),
                        "ledger_entries_count": hooks.food_ledger.entries().len()
                    },
                    "simulation_available": true,
                    "status": "completed"
                })
            );
            std::process::ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("SIMULATION_EXECUTION_FAILED: {e}");
            std::process::ExitCode::from(1)
        }
    }
}
