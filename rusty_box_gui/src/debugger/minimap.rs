//! The address-space minimap: a vertical strip laying every region of a
//! memory map top to bottom, on a log scale so a 4 KiB region stays visible
//! beside a 1 MiB one. Clicking it jumps the list to that region.
//!
//! The layout is pure (`lanes`, `address_at`), so it is tested without a
//! machine; the drawing lives in the page.

use super::memory_map::MemoryMap;

/// The least weight any region's lane gets, so the smallest region never
/// vanishes. Weights are in log2(size) units, not pixels.
pub(crate) const MIN_LANE_WEIGHT: f32 = 4.0;

/// One region's band in the strip.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MinimapLane {
    pub(crate) top: f32,
    pub(crate) height: f32,
    pub(crate) executable: bool,
}

/// A lane per region, in address order, filling `height` points. Each
/// region's share is `max(MIN_LANE_WEIGHT, log2(size))`, normalised — the log
/// compresses the address space so small and large regions are both visible.
pub(crate) fn lanes(map: &MemoryMap, height: f32) -> Vec<MinimapLane> {
    if map.regions.is_empty() || height <= 0.0 {
        return Vec::new();
    }
    let weight = |size: u64| MIN_LANE_WEIGHT.max((size.max(1) as f32).log2());
    let total: f32 = map.regions.iter().map(|region| weight(region.size)).sum();
    let mut top = 0.0;
    let mut lanes = Vec::with_capacity(map.regions.len());
    for (index, region) in map.regions.iter().enumerate() {
        // The last lane takes the exact remainder, so the strip ends at
        // `height` with no rounding gap.
        let lane_height = if index + 1 == map.regions.len() {
            (height - top).max(0.0)
        } else {
            weight(region.size) / total * height
        };
        lanes.push(MinimapLane {
            top,
            height: lane_height,
            executable: region.access.is_some_and(|access| access.executable),
        });
        top += lane_height;
    }
    lanes
}

/// The start address of the region the point `y` (in a strip `height` tall)
/// falls in, for click-to-jump.
pub(crate) fn address_at(map: &MemoryMap, height: f32, y: f32) -> Option<u64> {
    lanes(map, height)
        .iter()
        .zip(&map.regions)
        .find(|(lane, _)| y >= lane.top && y < lane.top + lane.height)
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
        assert!(lanes[0].height >= 20.0, "the small region keeps a visible band");
        assert!(lanes[1].executable);
        assert!(lanes[0].top >= 0.0);
        assert!(lanes[1].top + lanes[1].height <= 300.1);
        assert!(lanes[0].top < lanes[1].top);
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
}
