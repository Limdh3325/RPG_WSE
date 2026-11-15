//! 2D Tile map grid and collision/walkability boundary resource.

use bevy_ecs::prelude::Resource;
use game_domain::{MILLITILES_PER_TILE, millitiles_to_tile};

pub const DEFAULT_GRID_WIDTH_TILES: u32 = 64;
pub const DEFAULT_GRID_HEIGHT_TILES: u32 = 64;

/// 2D Map Grid representing terrain walkability and boundaries for a Region.
#[derive(Resource, Clone, Debug)]
pub struct MapGrid {
    width_tiles: u32,
    height_tiles: u32,
    blocked: Vec<bool>,
}

impl MapGrid {
    #[must_use]
    pub fn new(width_tiles: u32, height_tiles: u32) -> Self {
        let size = (width_tiles as usize) * (height_tiles as usize);
        Self {
            width_tiles,
            height_tiles,
            blocked: vec![false; size],
        }
    }

    #[must_use]
    pub fn default_64x64() -> Self {
        Self::new(DEFAULT_GRID_WIDTH_TILES, DEFAULT_GRID_HEIGHT_TILES)
    }

    #[must_use]
    pub const fn width_tiles(&self) -> u32 {
        self.width_tiles
    }

    #[must_use]
    pub const fn height_tiles(&self) -> u32 {
        self.height_tiles
    }

    #[must_use]
    pub const fn max_millitile_x(&self) -> i64 {
        (self.width_tiles as i64) * MILLITILES_PER_TILE
    }

    #[must_use]
    pub const fn max_millitile_y(&self) -> i64 {
        (self.height_tiles as i64) * MILLITILES_PER_TILE
    }

    #[must_use]
    pub fn is_in_bounds_tile(&self, tx: i64, ty: i64) -> bool {
        tx >= 0 && tx < (self.width_tiles as i64) && ty >= 0 && ty < (self.height_tiles as i64)
    }

    #[must_use]
    pub fn is_in_bounds_millitile(&self, mx: i64, my: i64) -> bool {
        mx >= 0 && mx < self.max_millitile_x() && my >= 0 && my < self.max_millitile_y()
    }

    pub fn set_tile_blocked(&mut self, tx: u32, ty: u32, blocked: bool) -> bool {
        if tx < self.width_tiles && ty < self.height_tiles {
            let index = (ty as usize) * (self.width_tiles as usize) + (tx as usize);
            self.blocked[index] = blocked;
            true
        } else {
            false
        }
    }

    #[must_use]
    pub fn is_tile_walkable(&self, tx: i64, ty: i64) -> bool {
        if !self.is_in_bounds_tile(tx, ty) {
            return false;
        }
        let index = (ty as usize) * (self.width_tiles as usize) + (tx as usize);
        !self.blocked[index]
    }

    #[must_use]
    pub fn is_millitile_walkable(&self, mx: i64, my: i64) -> bool {
        if !self.is_in_bounds_millitile(mx, my) {
            return false;
        }
        let tx = millitiles_to_tile(mx);
        let ty = millitiles_to_tile(my);
        self.is_tile_walkable(tx, ty)
    }
}

impl Default for MapGrid {
    fn default() -> Self {
        Self::default_64x64()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grid_boundaries() {
        let grid = MapGrid::default_64x64();
        assert_eq!(grid.width_tiles(), 64);
        assert_eq!(grid.height_tiles(), 64);
        assert_eq!(grid.max_millitile_x(), 64_000);
        assert_eq!(grid.max_millitile_y(), 64_000);

        assert!(grid.is_in_bounds_tile(0, 0));
        assert!(grid.is_in_bounds_tile(63, 63));
        assert!(!grid.is_in_bounds_tile(-1, 0));
        assert!(!grid.is_in_bounds_tile(0, -1));
        assert!(!grid.is_in_bounds_tile(64, 0));
        assert!(!grid.is_in_bounds_tile(0, 64));

        assert!(grid.is_in_bounds_millitile(0, 0));
        assert!(grid.is_in_bounds_millitile(63_999, 63_999));
        assert!(!grid.is_in_bounds_millitile(-1, 0));
        assert!(!grid.is_in_bounds_millitile(64_000, 0));
    }

    #[test]
    fn test_grid_walkability() {
        let mut grid = MapGrid::default_64x64();
        assert!(grid.is_tile_walkable(10, 10));
        assert!(grid.is_millitile_walkable(10_500, 10_500));

        grid.set_tile_blocked(10, 10, true);
        assert!(!grid.is_tile_walkable(10, 10));
        assert!(!grid.is_millitile_walkable(10_500, 10_500));
        assert!(grid.is_tile_walkable(11, 10));
    }
}
