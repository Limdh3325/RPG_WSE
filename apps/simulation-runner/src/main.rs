use clap::Parser;
use ecs_components::region_world::RegionWorld;
use game_domain::id::RegionId;
use simulation_core::engine::SimulationEngine;
use simulation_systems::economy::{FoodLedger, SettlementEconomy};
use simulation_systems::event_chain::CaravanEventChain;
use simulation_systems::faction::FactionManager;

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
    let world = RegionWorld::new_default_64x64(region_id, 1);
    let mut engine = SimulationEngine::new(world, args.seed);

    match engine.run_ticks(args.ticks) {
        Ok(summary) => {
            // Run settlement economy and dynamic event chains alongside ticks
            let mut settlement = SettlementEconomy::new_default(1);
            let mut ledger = FoodLedger::new();
            let mut faction_mgr = FactionManager::new();
            let mut active_caravan: Option<CaravanEventChain> = None;
            let mut total_economy_events = 0usize;

            for tick in 1..=args.ticks {
                let eco_events = settlement.step_economy(tick, 100, &mut ledger);
                total_economy_events += eco_events.len();

                if settlement.shortage_active && active_caravan.is_none() {
                    active_caravan = Some(CaravanEventChain::new_shortage_request(
                        format!("caravan-{}", tick),
                        settlement.settlement_id,
                        tick,
                        50,
                    ));
                }

                if let Some(mut caravan) = active_caravan.take() {
                    let chain_events = caravan.step(
                        tick,
                        &mut settlement,
                        &mut faction_mgr,
                        &mut ledger,
                        None,
                        None,
                    );
                    total_economy_events += chain_events.len();
                    if caravan.is_active() {
                        active_caravan = Some(caravan);
                    }
                }
            }

            println!(
                "{}",
                serde_json::json!({
                    "milestone": "M1",
                    "seed": summary.seed,
                    "requested_ticks": args.ticks,
                    "executed_ticks": summary.total_ticks_executed,
                    "final_state_hash": summary.final_state_hash,
                    "total_events_emitted": summary.total_events_emitted + total_economy_events,
                    "economy": {
                        "settlement_id": settlement.settlement_id,
                        "food_stock": settlement.food_stock,
                        "current_price": settlement.current_price,
                        "security_index": settlement.security_index,
                        "shortage_active": settlement.shortage_active,
                        "ledger_conservation_valid": ledger.audit_conservation(),
                        "ledger_entries_count": ledger.entries().len()
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
