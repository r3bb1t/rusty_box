//! The address-space minimap: a strip laying every region of a memory map out
//! in address order, on a log scale so a 4 KiB region stays visible beside a
//! 1 MiB one. Clicking it jumps the list to that region.
//!
//! The layout is pure (`lanes`, `address_at`), so it is tested without a
//! machine; the drawing lives in the page. The drawing and the click use the
//! same lanes, so what is drawn is what is hit.

use super::memory_map::MemoryMap;

/// The least weight any region's lane gets, so the smallest region never
/// vanishes. Weights are in log2(size) units, not points.
pub(crate) const MIN_LANE_WEIGHT: f32 = 4.0;

/// The least extent, in points, a lane is given while the strip has room for
/// every region at that extent; past that the lanes share it equally.
pub(crate) const MIN_LANE_EXTENT: f32 = 1.0;

/// One region's band in the strip.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MinimapLane {
    /// Where the band starts along the strip.
    pub(crate) offset: f32,
    /// How far the band runs along the strip.
    pub(crate) extent: f32,
    pub(crate) executable: bool,
}

/// A lane per region, in address order, filling `length` points. Each
/// region's share is `max(MIN_LANE_WEIGHT, log2(size))`, normalised — the log
/// compresses the address space so small and large regions are both visible —
/// and no lane is narrower than `MIN_LANE_EXTENT` unless the strip cannot
/// hold that many.
pub(crate) fn lanes(map: &MemoryMap, length: f32) -> Vec<MinimapLane> {
    if map.regions.is_empty() || length <= 0.0 {
        return Vec::new();
    }
    let weights: Vec<f32> = map
        .regions
        .iter()
        .map(|region| MIN_LANE_WEIGHT.max((region.size.max(1) as f32).log2()))
        .collect();
    let extents = extents(&weights, length);
    let mut offset = 0.0;
    map.regions
        .iter()
        .zip(extents)
        .map(|(region, extent)| {
            let lane = MinimapLane {
                offset,
                extent,
                executable: region.access.is_some_and(|access| access.executable),
            };
            offset += extent;
            lane
        })
        .collect()
}

/// Splits `length` over `weights` in proportion, giving none less than
/// `MIN_LANE_EXTENT`: a lane the proportion would narrow is held at the
/// minimum and the rest share what remains, until none falls below it. When
/// even the minimum does not fit, every lane gets an equal share.
fn extents(weights: &[f32], length: f32) -> Vec<f32> {
    let count = weights.len() as f32;
    if count * MIN_LANE_EXTENT >= length {
        return vec![length / count; weights.len()];
    }
    let mut held = vec![false; weights.len()];
    loop {
        let held_count = held.iter().filter(|&&held| held).count() as f32;
        let free_weight: f32 = weights
            .iter()
            .zip(&held)
            .filter(|(_, &held)| !held)
            .map(|(weight, _)| weight)
            .sum();
        let room = length - held_count * MIN_LANE_EXTENT;
        let mut newly_held = false;
        for (index, weight) in weights.iter().enumerate() {
            if !held[index] && weight / free_weight * room < MIN_LANE_EXTENT {
                held[index] = true;
                newly_held = true;
            }
        }
        if !newly_held {
            return weights
                .iter()
                .zip(&held)
                .map(|(weight, &held)| {
                    if held {
                        MIN_LANE_EXTENT
                    } else {
                        weight / free_weight * room
                    }
                })
                .collect();
        }
    }
}

/// The start address of the region the point `position` (in a strip `length`
/// long) falls in, for click-to-jump.
pub(crate) fn address_at(map: &MemoryMap, length: f32, position: f32) -> Option<u64> {
    lanes(map, length)
        .iter()
        .zip(&map.regions)
        .find(|(lane, _)| position >= lane.offset && position < lane.offset + lane.extent)
        .map(|(_, region)| region.start)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debugger::memory_map::{MapOrigin, MapRegion, RegionAccess, RegionOwner};

    fn region(start: u64, size: u64, executable: bool) -> MapRegion {
        MapRegion {
            start,
            size,
            resident_pages: size / 0x1000,
            total_pages: size / 0x1000,
            access: Some(RegionAccess {
                user: true,
                writable: false,
                executable,
                observed: true,
            }),
            owner: RegionOwner::Private,
            vad: None,
        }
    }

    fn map(regions: Vec<MapRegion>) -> MemoryMap {
        MemoryMap {
            origin: MapOrigin::PageTables,
            regions,
        }
    }

    #[test]
    fn lanes_keep_a_small_region_visible_beside_a_large_one() {
        let built = map(vec![
            region(0x1000, 0x1000, false),      // 4 KiB
            region(0x10_0000, 0x10_0000, true), // 1 MiB, executable
        ]);
        let lanes = lanes(&built, 300.0);
        assert_eq!(lanes.len(), 2);
        assert!(lanes[0].extent >= 20.0, "the small region keeps a visible band");
        assert!(lanes[1].executable);
        assert!(lanes[0].offset >= 0.0);
        assert!(lanes[1].offset + lanes[1].extent <= 300.1);
        assert!(lanes[0].offset < lanes[1].offset);
    }

    #[test]
    fn a_click_maps_to_the_region_it_lands_in() {
        let built = map(vec![
            region(0x1000, 0x1000, false),
            region(0x10_0000, 0x10_0000, true),
        ]);
        assert_eq!(address_at(&built, 300.0, 5.0), Some(0x1000));
        assert_eq!(address_at(&built, 300.0, 250.0), Some(0x10_0000));
        assert_eq!(address_at(&built, 300.0, 400.0), None);
    }

    /// Far more regions than points: every lane is the same width and the
    /// strip is exactly filled, so a click lands where a segment is drawn.
    #[test]
    fn more_regions_than_points_share_the_strip_equally() {
        let regions: Vec<MapRegion> = (0..2000u64)
            .map(|index| region(index * 0x2000, 0x1000 << (index % 8), false))
            .collect();
        let built = map(regions);
        let lanes = lanes(&built, 400.0);
        assert_eq!(lanes.len(), 2000);
        let last = lanes[1999];
        assert!((last.offset + last.extent - 400.0).abs() < 0.01);
        assert!(lanes.iter().all(|lane| (lane.extent - 0.2).abs() < 0.001));
        assert_eq!(address_at(&built, 400.0, 0.1), Some(0));
    }

    /// Few enough regions that each can keep the minimum: none is narrower,
    /// and the lanes still end at the strip's far edge.
    #[test]
    fn no_lane_is_narrower_than_the_minimum_while_it_fits() {
        let mut regions = vec![region(0x1000, 0x1000, false)];
        for index in 1..40u64 {
            regions.push(region(index * 0x1_0000, 0x40_0000_0000, false));
        }
        let built = map(regions);
        let lanes = lanes(&built, 100.0);
        assert!(lanes.iter().all(|lane| lane.extent >= MIN_LANE_EXTENT - 0.001));
        let last = lanes[lanes.len() - 1];
        assert!((last.offset + last.extent - 100.0).abs() < 0.01);
    }
}
