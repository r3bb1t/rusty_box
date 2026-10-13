//! A process's address space as a list of regions, each labelled by what
//! occupies it and how much of it is resident.
//!
//! The page-table map built here is the foundation: it walks the process's
//! own paging structures (`DebugCpu::dbg_page_mappings_in`) for the pages
//! actually mapped, and labels them with the modules the loader list names
//! and the sections their PE headers declare. A later pass can overlay
//! Windows' own region records (the VAD tree) to add reserved-but-unmapped
//! regions and Windows' protection and type; this map stands on its own
//! without it.
//!
//! The builder [`build`] takes plain data — the mappings and the module
//! layouts — so the region logic is tested without a machine.

use rusty_box::cpu::PageMapping;

use super::pe::PeSection;
use super::vad::{VadInfo, VadRegion};

const PAGE: u64 = 0x1000;

/// Where a map's regions came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MapOrigin {
    /// Only the process's page tables: reserved-but-unmapped memory is not
    /// seen, so a region's total is its mapped pages.
    PageTables,
    /// Windows' own records (the VAD tree), so reserved-but-unmapped regions
    /// appear and regions carry Windows' type and protection.
    WindowsRecords,
}

/// One region of the address space.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MapRegion {
    pub(crate) start: u64,
    pub(crate) size: u64,
    /// Pages of the region that are mapped.
    pub(crate) resident_pages: u64,
    /// Pages the region spans.
    pub(crate) total_pages: u64,
    /// What its pages grant, when any are mapped or a section's flags say.
    pub(crate) access: Option<RegionAccess>,
    pub(crate) owner: RegionOwner,
    /// Windows' own record of this region (type and protection), when the
    /// VAD tree was read and a VAD covers it.
    pub(crate) vad: Option<VadInfo>,
}

/// What a region's pages grant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RegionAccess {
    pub(crate) user: bool,
    pub(crate) writable: bool,
    pub(crate) executable: bool,
    /// The access was read from resident pages, not inferred from a section's
    /// PE flags.
    pub(crate) observed: bool,
}

/// Who a region belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RegionOwner {
    /// A module's PE header page.
    Header { base: u64, module: String },
    /// A module's section.
    Section {
        base: u64,
        module: String,
        section: String,
    },
    /// Mapped memory in no module: heaps, stacks, the PEB, mapped files.
    /// Named by thread and PEB walks in a later pass.
    Private,
}

/// A module as the map needs it: where it sits, its name, and its sections.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ModuleLayout {
    pub(crate) base: u64,
    pub(crate) name: String,
    pub(crate) sections: Vec<PeSection>,
}

/// A process's memory map.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MemoryMap {
    pub(crate) origin: MapOrigin,
    pub(crate) regions: Vec<MapRegion>,
}

/// One row of the memory map when its regions are grouped by module: a
/// module with its header and sections folded together, or a lone region.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MapGroup {
    Module {
        base: u64,
        name: String,
        /// Indices into `MemoryMap::regions`: the header, then the sections.
        regions: Vec<usize>,
        resident_pages: u64,
        total_pages: u64,
    },
    /// A private region, by its index in `MemoryMap::regions`.
    Region(usize),
}

/// The map's regions grouped by module, in address order: a run of a
/// module's header and section regions becomes one `Module`; a private
/// region becomes a `Region`.
pub(crate) fn group(map: &MemoryMap) -> Vec<MapGroup> {
    let mut groups: Vec<MapGroup> = Vec::new();
    for (index, region) in map.regions.iter().enumerate() {
        let (base, name) = match &region.owner {
            RegionOwner::Header { base, module } | RegionOwner::Section { base, module, .. } => {
                (*base, module.as_str())
            }
            RegionOwner::Private => {
                groups.push(MapGroup::Region(index));
                continue;
            }
        };
        match groups.last_mut() {
            Some(MapGroup::Module {
                base: group_base,
                regions,
                resident_pages,
                total_pages,
                ..
            }) if *group_base == base => {
                regions.push(index);
                *resident_pages += region.resident_pages;
                *total_pages += region.total_pages;
            }
            _ => groups.push(MapGroup::Module {
                base,
                name: name.to_owned(),
                regions: vec![index],
                resident_pages: region.resident_pages,
                total_pages: region.total_pages,
            }),
        }
    }
    groups
}

/// A labelled span the modules impose on the address space.
struct Label {
    start: u64,
    end: u64,
    owner: RegionOwner,
    /// A section's own PE flags, used when no page of it is resident.
    section_access: Option<RegionAccess>,
}

fn page_down(address: u64) -> u64 {
    address & !(PAGE - 1)
}

fn page_up(address: u64) -> u64 {
    address.wrapping_add(PAGE - 1) & !(PAGE - 1)
}

/// The spans the modules label, in address order and without overlap: a
/// header page, then one span per section.
fn module_labels(modules: &[ModuleLayout]) -> Vec<Label> {
    let mut labels = Vec::new();
    for module in modules {
        labels.push(Label {
            start: module.base,
            end: module.base + PAGE,
            owner: RegionOwner::Header {
                base: module.base,
                module: module.name.clone(),
            },
            section_access: None,
        });
        for section in &module.sections {
            let start = (module.base + u64::from(section.rva)).max(module.base + PAGE);
            let start = page_down(start);
            let end = page_up(module.base + u64::from(section.rva) + u64::from(section.virtual_size));
            if end <= start {
                continue;
            }
            labels.push(Label {
                start,
                end,
                owner: RegionOwner::Section {
                    base: module.base,
                    module: module.name.clone(),
                    section: section.name.clone(),
                },
                section_access: Some(RegionAccess {
                    user: true,
                    writable: section.writable,
                    executable: section.executable,
                    observed: false,
                }),
            });
        }
    }
    labels.sort_by_key(|label| label.start);
    labels
}

/// The combined access of every mapped page in `[lo, hi)`, and how many
/// there are.
fn resident(mappings: &[PageMapping], lo: u64, hi: u64) -> (u64, Option<RegionAccess>) {
    let mut pages = 0u64;
    let mut access: Option<RegionAccess> = None;
    for mapping in mappings {
        let start = mapping.linear.max(lo);
        let end = (mapping.linear + mapping.size).min(hi);
        if start >= end {
            continue;
        }
        pages += (end - start) / PAGE;
        let here = RegionAccess {
            user: mapping.access.user,
            writable: mapping.access.writable,
            executable: mapping.access.executable,
            observed: true,
        };
        access = Some(match access {
            None => here,
            Some(acc) => RegionAccess {
                user: acc.user || here.user,
                writable: acc.writable || here.writable,
                executable: acc.executable || here.executable,
                observed: true,
            },
        });
    }
    (pages, access)
}

/// Builds a process's page-table memory map from the mapped pages and the
/// module layouts. `mappings` are in address order, as the walk reports them.
pub(crate) fn build(mappings: &[PageMapping], modules: &[ModuleLayout]) -> MemoryMap {
    let labels = module_labels(modules);
    let mut regions = Vec::new();

    // Each module label becomes a region, residency read from the mappings.
    for label in &labels {
        let total_pages = (label.end - label.start) / PAGE;
        let (resident_pages, observed) = resident(mappings, label.start, label.end);
        let access = observed.or(label.section_access);
        regions.push(MapRegion {
            start: label.start,
            size: label.end - label.start,
            resident_pages,
            total_pages,
            access,
            owner: label.owner.clone(),
            vad: None,
        });
    }

    // Mapped pages in no label are private memory: contiguous runs of one
    // access become one region.
    let mut private = private_runs(mappings, &labels);
    regions.append(&mut private);

    regions.sort_by_key(|region| region.start);
    MemoryMap {
        origin: MapOrigin::PageTables,
        regions,
    }
}

/// The page-table map with Windows' VAD records overlaid: every region is
/// annotated with the VAD that covers its start, and each VAD that no mapped
/// region overlaps is added as a reserved row (resident 0), so reserved
/// memory and Windows' type and protection both show.
pub(crate) fn build_with_vads(
    mappings: &[PageMapping],
    modules: &[ModuleLayout],
    vads: &[VadRegion],
) -> MemoryMap {
    let mut map = build(mappings, modules);
    map.origin = MapOrigin::WindowsRecords;
    for region in &mut map.regions {
        region.vad = vads
            .iter()
            .find(|vad| vad.start <= region.start && region.start < vad.end)
            .map(|vad| vad.info);
    }
    for vad in vads {
        let overlaps = map
            .regions
            .iter()
            .any(|region| region.start < vad.end && region.start + region.size > vad.start);
        if !overlaps {
            map.regions.push(MapRegion {
                start: vad.start,
                size: vad.end - vad.start,
                resident_pages: 0,
                total_pages: (vad.end - vad.start) / PAGE,
                access: None,
                owner: RegionOwner::Private,
                vad: Some(vad.info),
            });
        }
    }
    map.regions.sort_by_key(|region| region.start);
    map
}

/// The parts of the mapped pages that no label covers, as private regions:
/// a run breaks at a gap in the mapping, at a label's edge, or where the
/// access changes.
fn private_runs(mappings: &[PageMapping], labels: &[Label]) -> Vec<MapRegion> {
    let mut regions = Vec::new();
    let mut run: Option<MapRegion> = None;
    for mapping in mappings {
        // Each 4 KiB page of the mapping, so a label edge inside a large
        // page splits it cleanly.
        let mut page = mapping.linear;
        let end = mapping.linear + mapping.size;
        let access = RegionAccess {
            user: mapping.access.user,
            writable: mapping.access.writable,
            executable: mapping.access.executable,
            observed: true,
        };
        while page < end {
            if labelled(labels, page) {
                if let Some(region) = run.take() {
                    regions.push(region);
                }
                page += PAGE;
                continue;
            }
            match &mut run {
                Some(region)
                    if region.start + region.size == page
                        && region.access == Some(access) =>
                {
                    region.size += PAGE;
                    region.resident_pages += 1;
                    region.total_pages += 1;
                }
                _ => {
                    if let Some(region) = run.take() {
                        regions.push(region);
                    }
                    run = Some(MapRegion {
                        start: page,
                        size: PAGE,
                        resident_pages: 1,
                        total_pages: 1,
                        access: Some(access),
                        owner: RegionOwner::Private,
                        vad: None,
                    });
                }
            }
            page += PAGE;
        }
    }
    if let Some(region) = run.take() {
        regions.push(region);
    }
    regions
}

/// Whether the page at `address` lies in any label.
fn labelled(labels: &[Label], address: u64) -> bool {
    labels
        .iter()
        .any(|label| label.start <= address && address < label.end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_box::cpu::PageAccess;

    fn map(linear: u64, size: u64, writable: bool, executable: bool) -> PageMapping {
        PageMapping {
            linear,
            physical: linear,
            size,
            access: PageAccess {
                user: true,
                writable,
                executable,
            },
        }
    }

    fn section(name: &str, rva: u32, size: u32, writable: bool, executable: bool) -> PeSection {
        PeSection {
            name: name.to_owned(),
            rva,
            virtual_size: size,
            raw_size: size,
            readable: true,
            writable,
            executable,
            code: executable,
        }
    }

    /// A module's header and its two sections become three regions, each with
    /// its residency and the access of its resident pages; a section with no
    /// resident page falls back to its PE flags.
    #[test]
    fn a_module_splits_into_header_and_section_regions() {
        let base = 0x0040_0000u64;
        let module = ModuleLayout {
            base,
            name: "app.exe".to_owned(),
            sections: vec![
                section(".text", 0x1000, 0x2000, false, true),
                section(".data", 0x3000, 0x1000, true, false),
            ],
        };
        // Header resident, one of .text's two pages resident, .data paged out.
        let mappings = [
            map(base, PAGE, false, false),
            map(base + 0x1000, PAGE, false, true),
        ];
        let regions = build(&mappings, std::slice::from_ref(&module)).regions;
        assert_eq!(regions.len(), 3);

        assert_eq!(
            regions[0].owner,
            RegionOwner::Header {
                base,
                module: "app.exe".to_owned()
            }
        );
        assert_eq!((regions[0].resident_pages, regions[0].total_pages), (1, 1));

        assert!(matches!(&regions[1].owner, RegionOwner::Section { section, .. } if section == ".text"));
        assert_eq!((regions[1].resident_pages, regions[1].total_pages), (1, 2));
        let text = regions[1].access.expect("resident");
        assert!(text.observed && text.executable && !text.writable);

        assert!(matches!(&regions[2].owner, RegionOwner::Section { section, .. } if section == ".data"));
        assert_eq!(regions[2].resident_pages, 0);
        let data = regions[2].access.expect("from PE flags");
        assert!(!data.observed && data.writable && !data.executable);
    }

    /// A module's header and sections fold into one group, private regions
    /// stay as lone rows between them, and the group sums residency.
    #[test]
    fn regions_group_by_module_keeping_private_rows_between() {
        let base = 0x0040_0000u64;
        let module = ModuleLayout {
            base,
            name: "app.exe".to_owned(),
            sections: vec![
                section(".text", 0x1000, 0x1000, false, true),
                section(".data", 0x2000, 0x1000, true, false),
            ],
        };
        let mappings = [
            map(0x1_0000, PAGE, true, false),
            map(base, PAGE, false, false),
            map(base + 0x1000, PAGE, false, true),
            map(base + 0x2000, PAGE, true, false),
            map(0x7000_0000, PAGE, true, false),
        ];
        let built = build(&mappings, std::slice::from_ref(&module));
        let groups = group(&built);
        assert_eq!(groups.len(), 3);
        assert!(matches!(groups[0], MapGroup::Region(_)));
        match &groups[1] {
            MapGroup::Module {
                base: group_base,
                name,
                regions,
                resident_pages,
                total_pages,
            } => {
                assert_eq!(*group_base, base);
                assert_eq!(name, "app.exe");
                assert_eq!(regions.len(), 3);
                assert_eq!((*resident_pages, *total_pages), (3, 3));
            }
            MapGroup::Region(_) => panic!("expected a module group"),
        }
        assert!(matches!(groups[2], MapGroup::Region(_)));
    }

    /// Mapped pages in no module are private; a run breaks where the access
    /// changes and where a module begins.
    #[test]
    fn mapped_memory_outside_modules_is_grouped_into_private_runs() {
        let base = 0x7000_0000u64;
        let module = ModuleLayout {
            base,
            name: "x.dll".to_owned(),
            sections: vec![section(".text", 0x1000, 0x1000, false, true)],
        };
        let mappings = [
            // Two writable private pages, then one read-only: two runs.
            map(0x10000, PAGE, true, false),
            map(0x11000, PAGE, true, false),
            map(0x12000, PAGE, false, false),
            // The module's header and section.
            map(base, PAGE, false, false),
            map(base + 0x1000, PAGE, false, true),
            // A private page past the module.
            map(base + 0x2000, PAGE, true, false),
        ];
        let regions = build(&mappings, std::slice::from_ref(&module)).regions;
        let private: Vec<&MapRegion> = regions
            .iter()
            .filter(|r| r.owner == RegionOwner::Private)
            .collect();
        assert_eq!(private.len(), 3);
        assert_eq!((private[0].start, private[0].size), (0x10000, 2 * PAGE));
        assert_eq!((private[1].start, private[1].size), (0x12000, PAGE));
        assert_eq!((private[2].start, private[2].size), (base + 0x2000, PAGE));
        // The whole map is in address order.
        assert!(regions.windows(2).all(|w| w[0].start <= w[1].start));
    }
}
