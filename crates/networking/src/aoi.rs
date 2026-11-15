//! Area of Interest (AOI) grid calculation.
//! Default cell size is 16x16 tiles, interest radius is 1 cell (3x3 grid).

use shared_protocol::EntityStateWire;
use std::collections::HashSet;
use uuid::Uuid;

pub const CELL_SIZE: i32 = 16;
pub const INTEREST_RADIUS_CELLS: i32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CellCoord {
    pub x: i32,
    pub y: i32,
}

impl CellCoord {
    #[must_use]
    pub fn from_world_pos(x: i32, y: i32) -> Self {
        Self {
            x: x.div_euclid(CELL_SIZE),
            y: y.div_euclid(CELL_SIZE),
        }
    }

    /// Returns the 9 cells centered on this cell (radius 1).
    #[must_use]
    pub fn interest_cells(&self) -> Vec<CellCoord> {
        let mut cells = Vec::with_capacity(9);
        for dy in -INTEREST_RADIUS_CELLS..=INTEREST_RADIUS_CELLS {
            for dx in -INTEREST_RADIUS_CELLS..=INTEREST_RADIUS_CELLS {
                cells.push(CellCoord {
                    x: self.x + dx,
                    y: self.y + dy,
                });
            }
        }
        cells
    }

    #[must_use]
    pub fn is_adjacent_or_equal(&self, other: &CellCoord) -> bool {
        (self.x - other.x).abs() <= INTEREST_RADIUS_CELLS
            && (self.y - other.y).abs() <= INTEREST_RADIUS_CELLS
    }
}

pub struct AoiFilter;

impl AoiFilter {
    /// Returns true if `target` is within the viewer's 3x3 AOI window.
    #[must_use]
    pub fn is_in_interest(viewer_x: i32, viewer_y: i32, target_x: i32, target_y: i32) -> bool {
        let vc = CellCoord::from_world_pos(viewer_x, viewer_y);
        let tc = CellCoord::from_world_pos(target_x, target_y);
        vc.is_adjacent_or_equal(&tc)
    }

    /// Filters a list of entities to only those within the viewer's AOI.
    #[must_use]
    pub fn filter_entities(
        viewer_x: i32,
        viewer_y: i32,
        entities: &[EntityStateWire],
    ) -> Vec<EntityStateWire> {
        let vc = CellCoord::from_world_pos(viewer_x, viewer_y);
        entities
            .iter()
            .filter(|e| {
                let tc = CellCoord::from_world_pos(e.x, e.y);
                vc.is_adjacent_or_equal(&tc)
            })
            .cloned()
            .collect()
    }

    /// Calculates entity enters and leaves when a viewer moves between cells.
    #[must_use]
    pub fn calculate_cell_transition(
        old_cell: CellCoord,
        new_cell: CellCoord,
        all_entities: &[EntityStateWire],
    ) -> (Vec<EntityStateWire>, Vec<Uuid>) {
        if old_cell == new_cell {
            return (Vec::new(), Vec::new());
        }

        let old_interest: HashSet<CellCoord> = old_cell.interest_cells().into_iter().collect();
        let new_interest: HashSet<CellCoord> = new_cell.interest_cells().into_iter().collect();

        let mut entered = Vec::new();
        let mut left = Vec::new();

        for e in all_entities {
            let tc = CellCoord::from_world_pos(e.x, e.y);
            let in_old = old_interest.contains(&tc);
            let in_new = new_interest.contains(&tc);

            if !in_old && in_new {
                entered.push(e.clone());
            } else if in_old && !in_new {
                left.push(e.id);
            }
        }

        (entered, left)
    }
}
