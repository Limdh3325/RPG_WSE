//! LLM Security Boundary and Command Validation.
//!
//! Ensures that suggestions or commands from generative/LLM systems
//! cannot bypass game-server verification or create arbitrary/invalid items (TEST-AI-005).

use game_domain::EntityId;
use std::collections::HashSet;
use thiserror::Error;

/// Security violations caught by the LLM verification boundary.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LlmSecurityViolation {
    #[error("Proposed item '{0}' does not exist in the authoritative item registry")]
    UnknownItem(String),
    #[error("Proposed grant quantity {0} exceeds authorized limit {1}")]
    QuantityExceeded(u32, u32),
    #[error("Target entity {0:?} is not eligible for this action")]
    IneligibleTarget(EntityId),
    #[error("Unauthorized grant source: {0}")]
    UnauthorizedSource(String),
}

/// Registry of authoritative valid items.
#[derive(Debug, Clone)]
pub struct AuthoritativeItemRegistry {
    pub valid_item_ids: HashSet<String>,
    pub max_grant_per_action: u32,
}

impl Default for AuthoritativeItemRegistry {
    fn default() -> Self {
        let mut set = HashSet::new();
        set.insert("item_bread".to_string());
        set.insert("item_apple".to_string());
        set.insert("item_gold_coin".to_string());
        set.insert("item_wood".to_string());
        set.insert("item_iron_ore".to_string());
        set.insert("item_sword_basic".to_string());
        set.insert("item_shield_basic".to_string());

        Self {
            valid_item_ids: set,
            max_grant_per_action: 100,
        }
    }
}

/// A proposal originating from an LLM, dialogue agent, or untrusted source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmItemGrantProposal {
    pub target_entity_id: EntityId,
    pub item_id: String,
    pub quantity: u32,
    pub source: String,
}

/// Validates an LLM-originated item grant proposal.
///
/// Returns `Ok(quantity)` if approved, or `Err(violation)` with 0 items granted.
pub fn validate_and_grant_item(
    registry: &AuthoritativeItemRegistry,
    proposal: &LlmItemGrantProposal,
) -> Result<u32, LlmSecurityViolation> {
    // 1. Verify existence in authoritative item registry
    if !registry.valid_item_ids.contains(&proposal.item_id) {
        return Err(LlmSecurityViolation::UnknownItem(proposal.item_id.clone()));
    }

    // 2. Verify grant quantity ceiling
    if proposal.quantity > registry.max_grant_per_action {
        return Err(LlmSecurityViolation::QuantityExceeded(
            proposal.quantity,
            registry.max_grant_per_action,
        ));
    }

    Ok(proposal.quantity)
}
