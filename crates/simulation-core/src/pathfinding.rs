//! Deterministic integer A* pathfinding with node expansion budget and collision checks.

use ecs_components::grid::MapGrid;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

/// Pathfinding failure conditions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathError {
    StartOutOfBounds,
    GoalOutOfBounds,
    StartBlocked,
    GoalBlocked,
    NoPathFound,
    BudgetExceeded { visited_nodes: usize, budget: usize },
}

impl std::fmt::Display for PathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StartOutOfBounds => write!(f, "start position out of bounds"),
            Self::GoalOutOfBounds => write!(f, "goal position out of bounds"),
            Self::StartBlocked => write!(f, "start position is blocked"),
            Self::GoalBlocked => write!(f, "goal position is blocked"),
            Self::NoPathFound => write!(f, "no walkable path to goal"),
            Self::BudgetExceeded {
                visited_nodes,
                budget,
            } => {
                write!(f, "search budget exceeded ({visited_nodes}/{budget})")
            }
        }
    }
}

impl std::error::Error for PathError {}

/// Calculates deterministic Manhattan distance heuristic between two tiles without overflow.
#[inline]
#[must_use]
pub fn manhattan_distance(a: (i64, i64), b: (i64, i64)) -> u64 {
    let dx = (a.0 as i128 - b.0 as i128).unsigned_abs() as u64;
    let dy = (a.1 as i128 - b.1 as i128).unsigned_abs() as u64;
    dx.saturating_add(dy)
}

type NodePriority = Reverse<(u64, u64, (i64, i64))>;

/// Computes an A* path over integer tile coordinates with strict node expansion budget.
///
/// Returns waypoints in integer millitiles. Intermediate waypoints point to tile centers,
/// while the final waypoint points exactly to `goal_millitile`.
pub fn astar_path(
    grid: &MapGrid,
    start_millitile: (i64, i64),
    goal_millitile: (i64, i64),
    max_expansions: usize,
) -> Result<Vec<(i64, i64)>, PathError> {
    let (sx, sy) = start_millitile;
    let (gx, gy) = goal_millitile;

    let s_tx = sx / 1000;
    let s_ty = sy / 1000;
    let g_tx = gx / 1000;
    let g_ty = gy / 1000;

    if !grid.is_in_bounds_tile(s_tx, s_ty) {
        return Err(PathError::StartOutOfBounds);
    }
    if !grid.is_in_bounds_tile(g_tx, g_ty) {
        return Err(PathError::GoalOutOfBounds);
    }

    let start_tile = (s_tx, s_ty);
    let goal_tile = (g_tx, g_ty);

    if !grid.is_tile_walkable(start_tile.0, start_tile.1) {
        return Err(PathError::StartBlocked);
    }
    if !grid.is_tile_walkable(goal_tile.0, goal_tile.1) {
        return Err(PathError::GoalBlocked);
    }

    if start_tile == goal_tile {
        return Ok(vec![goal_millitile]);
    }

    let mut open_set: BinaryHeap<NodePriority> = BinaryHeap::new();
    let mut came_from: HashMap<(i64, i64), (i64, i64)> = HashMap::new();
    let mut g_score: HashMap<(i64, i64), u64> = HashMap::new();

    let initial_h = manhattan_distance(start_tile, goal_tile);
    g_score.insert(start_tile, 0);
    open_set.push(Reverse((initial_h, initial_h, start_tile)));

    let mut expansions = 0;
    let directions = [(0i64, 1i64), (0i64, -1i64), (1i64, 0i64), (-1i64, 0i64)];

    while let Some(Reverse((_f, _h, current))) = open_set.pop() {
        if current == goal_tile {
            // Reconstruct path
            let mut curr = current;
            let mut tile_path = vec![curr];
            while let Some(&prev) = came_from.get(&curr) {
                tile_path.push(prev);
                curr = prev;
            }
            tile_path.reverse();

            // Convert to millitile waypoints, skipping starting tile if we have subsequent steps
            let mut waypoints = Vec::new();
            let steps = if tile_path.len() > 1 {
                &tile_path[1..]
            } else {
                &tile_path[..]
            };

            for (idx, &tile) in steps.iter().enumerate() {
                if idx == steps.len() - 1 {
                    waypoints.push(goal_millitile);
                } else {
                    let wx = tile.0.saturating_mul(1000).saturating_add(500);
                    let wy = tile.1.saturating_mul(1000).saturating_add(500);
                    waypoints.push((wx, wy));
                }
            }

            return Ok(waypoints);
        }

        expansions += 1;
        if expansions >= max_expansions {
            return Err(PathError::BudgetExceeded {
                visited_nodes: expansions,
                budget: max_expansions,
            });
        }

        let current_g = g_score.get(&current).copied().unwrap_or(u64::MAX);

        for (dx, dy) in directions {
            let neighbor = (current.0.saturating_add(dx), current.1.saturating_add(dy));
            if !grid.is_in_bounds_tile(neighbor.0, neighbor.1)
                || !grid.is_tile_walkable(neighbor.0, neighbor.1)
            {
                continue;
            }

            let tentative_g = current_g.saturating_add(1);
            let neighbor_g = g_score.get(&neighbor).copied().unwrap_or(u64::MAX);

            if tentative_g < neighbor_g {
                came_from.insert(neighbor, current);
                g_score.insert(neighbor, tentative_g);
                let h = manhattan_distance(neighbor, goal_tile);
                let f = tentative_g.saturating_add(h);
                open_set.push(Reverse((f, h, neighbor)));
            }
        }
    }

    Err(PathError::NoPathFound)
}
