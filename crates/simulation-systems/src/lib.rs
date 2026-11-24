//! Movement, combat, NPC Utility AI / GOAP rules, and simulation systems.
//!
//! Part of the RPG World Simulation Engine (WSE).

pub mod economy;
pub mod event_chain;
pub mod faction;
pub mod goap;
pub mod integrated_hooks;
pub mod llm_boundary;
pub mod memory;
pub mod needs;
pub mod perception;
pub mod prologue;
pub mod rule_dsl;
pub mod scheduler;
pub mod system;
pub mod utility;

pub use economy::{FoodLedger, FoodLedgerEntry, FoodLedgerOp, SettlementEconomy};
pub use event_chain::{
    CaravanEventChain, CaravanPhase, CaravanResolution, PlayerCaravanIntervention,
};
pub use faction::{FactionId, FactionManager, FactionTier};
pub use goap::{GoapActionId, GoapGoal, GoapPlanner, GoapPlannerConfig, GoapState};
pub use integrated_hooks::IntegratedSimulationHooks;
pub use llm_boundary::{
    AuthoritativeItemRegistry, LlmItemGrantProposal, LlmSecurityViolation, validate_and_grant_item,
};
pub use memory::MemoryManager;
pub use needs::update_needs;
pub use perception::{
    NpcWorldFacts, PerceptionConfig, PerceptionContext, SurroundingEntityData, extract_world_facts,
};
pub use prologue::{
    CharacterPrologueSession, PrologueAction, PrologueError, PrologueManager,
    PrologueRewardReceipt, PrologueState, PrologueStep, PrologueTransferRecord, TransferRegistry,
    TransferStatus,
};
pub use rule_dsl::{
    ActionAST, ComparisonOp, ConditionAST, DynamicRule, DynamicRuleRegistry, MetricContext,
    MetricKey,
};
pub use scheduler::{NpcScheduler, NpcSchedulerConfig};
pub use system::{NpcAgentState, NpcAiLifecycleState, NpcAiSystem, NpcPendingCommand};
pub use utility::{NpcGoal, UtilityEvaluator};
