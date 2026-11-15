//! Coordinate system definitions and unit conversions.
//! 1 tile = 1,000 millitiles. Z-axis is excluded.

/// Fixed ratio of millitiles per grid tile.
pub const MILLITILES_PER_TILE: i64 = 1000;

/// Converts millitiles to the containing tile index.
#[must_use]
pub const fn millitiles_to_tile(millitiles: i64) -> i64 {
    if millitiles >= 0 {
        millitiles / MILLITILES_PER_TILE
    } else {
        (millitiles - MILLITILES_PER_TILE + 1) / MILLITILES_PER_TILE
    }
}

/// Converts a tile index to the origin millitile coordinate (minimum coordinate for the tile).
#[must_use]
pub const fn tile_to_origin_millitile(tile: i64) -> i64 {
    tile * MILLITILES_PER_TILE
}

/// Converts a tile index to the center millitile coordinate.
#[must_use]
pub const fn tile_to_center_millitile(tile: i64) -> i64 {
    tile * MILLITILES_PER_TILE + (MILLITILES_PER_TILE / 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_millitile_to_tile_positive() {
        assert_eq!(millitiles_to_tile(0), 0);
        assert_eq!(millitiles_to_tile(500), 0);
        assert_eq!(millitiles_to_tile(999), 0);
        assert_eq!(millitiles_to_tile(1000), 1);
        assert_eq!(millitiles_to_tile(1450), 1);
        assert_eq!(millitiles_to_tile(63999), 63);
        assert_eq!(millitiles_to_tile(64000), 64);
    }

    #[test]
    fn test_millitile_to_tile_negative() {
        assert_eq!(millitiles_to_tile(-1), -1);
        assert_eq!(millitiles_to_tile(-1000), -1);
        assert_eq!(millitiles_to_tile(-1001), -2);
    }

    #[test]
    fn test_tile_to_millitiles() {
        assert_eq!(tile_to_origin_millitile(0), 0);
        assert_eq!(tile_to_center_millitile(0), 500);
        assert_eq!(tile_to_origin_millitile(10), 10_000);
        assert_eq!(tile_to_center_millitile(10), 10_500);
    }
}
