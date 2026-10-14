//! A process's address space as a list of regions, each labelled by what
//! occupies it and how much of it is resident.
//!
//! The page-table map built here is the foundation: it walks the process's
//! own paging structures (`DebugCpu::dbg_page_mappings_in`) for the pages
//! actually mapped, and labels them with the modules the loader list names
//! and the sections their PE headers declare. Windows' own region records
//! (the VAD tree) can be overlaid to add reserved-but-unmapped regions and
//! Windows' protection and type; the map stands on its own without them.
//!
//! The builders [`build`] and [`build_with_vads`] take plain data — the
//! mappings (in address order, as the walk reports them) and the module
//! layouts — so the region logic is tested without a machine.

use std::collections::BTreeMap;

use rusty_box::cpu::{LinearRange, PageMapping};

use super::pe::PeSection;
use super::threads::{stack_owner, WinThread};
use super::vad::{VadInfo, VadKind, VadRegion};

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
    /// VAD tree was read and a VAD covers its start. A region never spans two
    /// VADs.
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
    /// A thread's user stack, named from its `TEB` by [`label_stacks`].
    Stack { tid: u64 },
    /// Mapped or reserved memory in no module: heaps, the PEB, mapped files.
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

/// The map's regions grouped by module, in address order: all of a module's
/// header and section regions become one `Module` at the module's first
/// region, however much private memory lies between them; every other region
/// becomes a `Region`.
pub(crate) fn group(map: &MemoryMap) -> Vec<MapGroup> {
    let mut groups: Vec<MapGroup> = Vec::new();
    let mut by_base: BTreeMap<u64, usize> = BTreeMap::new();
    for (index, region) in map.regions.iter().enumerate() {
        let (base, name) = match &region.owner {
            RegionOwner::Header { base, module } | RegionOwner::Section { base, module, .. } => {
                (*base, module.as_str())
            }
            RegionOwner::Stack { .. } | RegionOwner::Private => {
                groups.push(MapGroup::Region(index));
                continue;
            }
        };
        match by_base.get(&base).and_then(|&at| groups.get_mut(at)) {
            Some(MapGroup::Module {
                regions,
                resident_pages,
                total_pages,
                ..
            }) => {
                regions.push(index);
                *resident_pages += region.resident_pages;
                *total_pages += region.total_pages;
            }
            Some(MapGroup::Region(_)) | None => {
                by_base.insert(base, groups.len());
                groups.push(MapGroup::Module {
                    base,
                    name: name.to_owned(),
                    regions: vec![index],
                    resident_pages: region.resident_pages,
                    total_pages: region.total_pages,
                });
            }
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

/// A half-open span of addresses, `start..end`.
#[derive(Clone, Copy)]
struct Span {
    start: u64,
    end: u64,
}

fn page_down(address: u64) -> u64 {
    address & !(PAGE - 1)
}

fn page_up(address: u64) -> u64 {
    address.wrapping_add(PAGE - 1) & !(PAGE - 1)
}

/// The spans the modules label, in address order: a header page, then one
/// span per section. A module or section whose extent would run past the top
/// of the address space is not labelled rather than wrapped.
fn module_labels(modules: &[ModuleLayout]) -> Vec<Label> {
    let mut labels = Vec::new();
    for module in modules {
        let Some(header_end) = module.base.checked_add(PAGE) else {
            continue;
        };
        labels.push(Label {
            start: module.base,
            end: header_end,
            owner: RegionOwner::Header {
                base: module.base,
                module: module.name.clone(),
            },
            section_access: None,
        });
        for section in &module.sections {
            // The loader sizes a section by VirtualSize, and by SizeOfRawData
            // when VirtualSize is zero.
            let size = if section.virtual_size == 0 {
                section.raw_size
            } else {
                section.virtual_size
            };
            let first = module.base.saturating_add(u64::from(section.rva));
            let Some(last) = first.checked_add(u64::from(size)) else {
                continue;
            };
            let start = page_down(first.max(header_end));
            let end = page_up(last);
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

/// The labels' spans merged into disjoint spans in address order, for
/// finding the label a page lies in.
fn label_spans(labels: &[Label]) -> Vec<Span> {
    let mut spans: Vec<Span> = Vec::new();
    for label in labels {
        match spans.last_mut() {
            Some(last) if label.start <= last.end => last.end = last.end.max(label.end),
            _ => spans.push(Span {
                start: label.start,
                end: label.end,
            }),
        }
    }
    spans
}

/// The span of `spans` (disjoint, in address order) holding `address`.
fn span_at(spans: &[Span], address: u64) -> Option<Span> {
    let after = spans.partition_point(|span| span.start <= address);
    after
        .checked_sub(1)
        .map(|at| spans[at])
        .filter(|span| address < span.end)
}

/// The index of the VAD of `vads` (disjoint, in address order) holding
/// `address`.
fn vad_covering(vads: &[VadRegion], address: u64) -> Option<usize> {
    let after = vads.partition_point(|vad| vad.start <= address);
    after
        .checked_sub(1)
        .filter(|&at| address < vads[at].end)
}

/// The combined access of every mapped page in `[lo, hi)`, and how many
/// there are. `mappings` are in address order and do not overlap.
fn resident(mappings: &[PageMapping], lo: u64, hi: u64) -> (u64, Option<RegionAccess>) {
    let first = mappings.partition_point(|mapping| mapping.linear.saturating_add(mapping.size) <= lo);
    let mut pages = 0u64;
    let mut access: Option<RegionAccess> = None;
    for mapping in &mappings[first..] {
        if mapping.linear >= hi {
            break;
        }
        let start = mapping.linear.max(lo);
        let end = mapping.linear.saturating_add(mapping.size).min(hi);
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

/// The parts of `mappings` that lie inside `range`, each cut at the range's
/// ends with its physical address moved along with its start. A walk reports
/// a large page whole, so one that begins inside a module can run on past its
/// end; cut, it counts only the module's own pages.
pub(crate) fn clip_mappings(mappings: &[PageMapping], range: LinearRange) -> Vec<PageMapping> {
    mappings
        .iter()
        .filter_map(|mapping| {
            let start = mapping.linear.max(range.start);
            let end = mapping.linear.saturating_add(mapping.size).min(range.end);
            (start < end).then(|| PageMapping {
                linear: start,
                physical: mapping.physical.wrapping_add(start - mapping.linear),
                size: end - start,
                access: mapping.access,
            })
        })
        .collect()
}

/// Builds a process's page-table memory map from the mapped pages and the
/// module layouts. `mappings` are in address order, as the walk reports them.
pub(crate) fn build(mappings: &[PageMapping], modules: &[ModuleLayout]) -> MemoryMap {
    assemble(mappings, modules, None)
}

/// The page-table map with Windows' VAD records overlaid. Mapped memory is
/// split where one VAD ends and the next begins, so every region carries the
/// type and protection of the one VAD it lies in; each stretch of a VAD that
/// no mapped region covers is added as a reserved row (resident 0). An image
/// view whose pages a listed module's sections cover adds none: the module
/// owns its padding.
pub(crate) fn build_with_vads(
    mappings: &[PageMapping],
    modules: &[ModuleLayout],
    vads: &[VadRegion],
) -> MemoryMap {
    let mut ordered = vads.to_vec();
    ordered.sort_by_key(|vad| vad.start);
    assemble(mappings, modules, Some(&ordered))
}

fn assemble(
    mappings: &[PageMapping],
    modules: &[ModuleLayout],
    vads: Option<&[VadRegion]>,
) -> MemoryMap {
    let records = vads.unwrap_or(&[]);
    let labels = module_labels(modules);
    let spans = label_spans(&labels);
    let mut regions = Vec::with_capacity(labels.len());

    // Each module label becomes a region, residency read from the mappings.
    for label in &labels {
        let total_pages = (label.end - label.start) / PAGE;
        let (resident_pages, observed) = resident(mappings, label.start, label.end);
        regions.push(MapRegion {
            start: label.start,
            size: label.end - label.start,
            resident_pages,
            total_pages,
            access: observed.or(label.section_access),
            owner: label.owner.clone(),
            vad: vad_covering(records, label.start).map(|at| records[at].info),
        });
    }

    // Mapped pages in no label are private memory: contiguous runs of one
    // access inside one VAD become one region.
    regions.append(&mut private_runs(mappings, &spans, records));
    regions.sort_by_key(|region| region.start);

    let origin = match vads {
        Some(_) => {
            let mut reserved = reserved_regions(&regions, records);
            regions.append(&mut reserved);
            regions.sort_by_key(|region| region.start);
            MapOrigin::WindowsRecords
        }
        None => MapOrigin::PageTables,
    };
    MemoryMap { origin, regions }
}

/// The stretches of each VAD that no region of `regions` (in address order)
/// covers, as reserved rows carrying the VAD's record.
fn reserved_regions(regions: &[MapRegion], vads: &[VadRegion]) -> Vec<MapRegion> {
    let mut reserved = Vec::new();
    for vad in vads {
        let first = regions
            .partition_point(|region| region.start.saturating_add(region.size) <= vad.start);
        let mut cursor = vad.start;
        let mut module_owned = false;
        let mut gaps: Vec<Span> = Vec::new();
        for region in &regions[first..] {
            if region.start >= vad.end {
                break;
            }
            if matches!(
                region.owner,
                RegionOwner::Header { .. } | RegionOwner::Section { .. }
            ) {
                module_owned = true;
            }
            if region.start > cursor {
                gaps.push(Span {
                    start: cursor,
                    end: region.start,
                });
            }
            cursor = cursor.max(region.start.saturating_add(region.size));
        }
        if cursor < vad.end {
            gaps.push(Span {
                start: cursor,
                end: vad.end,
            });
        }
        if vad.info.kind == VadKind::Image && module_owned {
            continue;
        }
        for gap in gaps {
            let size = gap.end - gap.start;
            reserved.push(MapRegion {
                start: gap.start,
                size,
                resident_pages: 0,
                total_pages: size / PAGE,
                access: None,
                owner: RegionOwner::Private,
                vad: Some(vad.info),
            });
        }
    }
    reserved
}

/// Renames each private region that lies wholly inside a thread's user stack to
/// `Stack { tid }`. A module header or section keeps its name; only unowned
/// private memory can be a stack.
pub(crate) fn label_stacks(map: &mut MemoryMap, threads: &[WinThread]) {
    for region in &mut map.regions {
        if !matches!(region.owner, RegionOwner::Private) {
            continue;
        }
        if let Some(tid) = stack_owner(region.start, region.start + region.size, threads) {
            region.owner = RegionOwner::Stack { tid };
        }
    }
}

/// A run of mapped private pages being built.
struct OpenRun {
    region: MapRegion,
    /// The VAD, by index, every page of the run lies in.
    vad: Option<usize>,
}

/// The parts of the mapped pages that no label covers, as private regions:
/// a run breaks at a gap in the mapping, at a label's edge, at a VAD's edge,
/// or where the access changes.
fn private_runs(mappings: &[PageMapping], spans: &[Span], vads: &[VadRegion]) -> Vec<MapRegion> {
    let mut regions = Vec::new();
    let mut run: Option<OpenRun> = None;
    for mapping in mappings {
        // Each 4 KiB page of the mapping, so a label or VAD edge inside a
        // large page splits it cleanly.
        let mut page = mapping.linear;
        let end = mapping.linear.saturating_add(mapping.size);
        let access = RegionAccess {
            user: mapping.access.user,
            writable: mapping.access.writable,
            executable: mapping.access.executable,
            observed: true,
        };
        while page < end {
            if let Some(span) = span_at(spans, page) {
                if let Some(open) = run.take() {
                    regions.push(open.region);
                }
                page = span.end;
                continue;
            }
            let vad = vad_covering(vads, page);
            match &mut run {
                Some(open)
                    if open.region.start + open.region.size == page
                        && open.region.access == Some(access)
                        && open.vad == vad =>
                {
                    open.region.size += PAGE;
                    open.region.resident_pages += 1;
                    open.region.total_pages += 1;
                }
                _ => {
                    if let Some(open) = run.take() {
                        regions.push(open.region);
                    }
                    run = Some(OpenRun {
                        region: MapRegion {
                            start: page,
                            size: PAGE,
                            resident_pages: 1,
                            total_pages: 1,
                            access: Some(access),
                            owner: RegionOwner::Private,
                            vad: vad.map(|at| vads[at].info),
                        },
                        vad,
                    });
                }
            }
            page += PAGE;
        }
    }
    if let Some(open) = run.take() {
        regions.push(open.region);
    }
    regions
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debugger::vad::WinProtection;
    use rusty_box::cpu::PageAccess;

    /// A large page that runs past a module's end is cut at the end, its
    /// physical address moved with its start; a mapping wholly outside goes.
    #[test]
    fn mappings_are_cut_to_the_range_asked_for() {
        let range = LinearRange { start: 0x40_0000, end: 0x48_0000 };
        let large = map(0x40_0000, 0x20_0000, true, true);
        let before = map(0x3F_F000, 0x2000, false, false);
        let outside = map(0x60_0000, 0x1000, false, false);
        let cut = clip_mappings(&[before, large, outside], range);
        assert_eq!(cut.len(), 2);
        assert_eq!((cut[0].linear, cut[0].size, cut[0].physical), (0x40_0000, 0x1000, 0x40_0000));
        assert_eq!((cut[1].linear, cut[1].size), (0x40_0000, 0x8_0000));
    }

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

    fn vad(start: u64, end: u64, kind: VadKind) -> VadRegion {
        VadRegion {
            start,
            end,
            info: VadInfo {
                kind,
                protection: Some(WinProtection::ReadWrite),
            },
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

    /// A reservation with a few committed pages: the committed pages are one
    /// region and the reserved remainder above and below are reserved rows,
    /// all carrying the VAD's record.
    #[test]
    fn a_vad_with_committed_pages_shows_its_reserved_remainder() {
        let mappings = [map(0x20_0000, 3 * PAGE, true, false)];
        let vads = [vad(0x10_0000, 0x30_0000, VadKind::Private)];
        let built = build_with_vads(&mappings, &[], &vads);
        assert_eq!(built.origin, MapOrigin::WindowsRecords);
        let shape: Vec<(u64, u64, u64)> = built
            .regions
            .iter()
            .map(|r| (r.start, r.size, r.resident_pages))
            .collect();
        assert_eq!(
            shape,
            [
                (0x10_0000, 0x10_0000, 0),
                (0x20_0000, 3 * PAGE, 3),
                (0x20_3000, 0x10_0000 - 3 * PAGE, 0),
            ]
        );
        assert!(built.regions.iter().all(|r| r.vad.map(|v| v.kind) == Some(VadKind::Private)));
    }

    /// Two adjacent VADs whose pages carry the same rights are two regions,
    /// each with its own VAD's type — not one run labelled by the first.
    #[test]
    fn adjacent_vads_with_the_same_page_rights_stay_separate() {
        let mappings = [map(0x40_0000, 4 * PAGE, true, false)];
        let vads = [
            vad(0x40_0000, 0x40_2000, VadKind::Private),
            vad(0x40_2000, 0x40_4000, VadKind::Mapped),
        ];
        let built = build_with_vads(&mappings, &[], &vads);
        assert_eq!(built.regions.len(), 2);
        assert_eq!(built.regions[0].vad.map(|v| v.kind), Some(VadKind::Private));
        assert_eq!(built.regions[0].size, 2 * PAGE);
        assert_eq!(built.regions[1].vad.map(|v| v.kind), Some(VadKind::Mapped));
        assert_eq!(built.regions[1].start, 0x40_2000);
    }

    /// An image view that a listed module's sections cover adds no reserved
    /// rows for its non-resident pages: the module row already accounts for
    /// them. An image view no module claims does, so an unlisted image shows.
    #[test]
    fn an_image_vad_adds_reserved_rows_only_when_no_listed_module_owns_it() {
        let base = 0x0040_0000u64;
        let module = ModuleLayout {
            base,
            name: "app.exe".to_owned(),
            sections: vec![section(".text", 0x1000, 0x2000, false, true)],
        };
        let mappings = [map(base, PAGE, false, false)];
        let vads = [
            vad(base, base + 0x4000, VadKind::Image),
            vad(0x5000_0000, 0x5000_8000, VadKind::Image),
        ];
        let built = build_with_vads(&mappings, std::slice::from_ref(&module), &vads);
        // Header + .text for the module; one reserved row for the unlisted image.
        let reserved: Vec<&MapRegion> = built
            .regions
            .iter()
            .filter(|r| r.owner == RegionOwner::Private)
            .collect();
        assert_eq!(reserved.len(), 1);
        assert_eq!((reserved[0].start, reserved[0].size), (0x5000_0000, 0x8000));
        assert_eq!(reserved[0].vad.map(|v| v.kind), Some(VadKind::Image));
        assert_eq!(built.regions.len(), 3);
    }

    /// A section with VirtualSize 0 is sized by SizeOfRawData, as the loader
    /// does, so its pages stay in the module instead of becoming private runs.
    #[test]
    fn a_section_with_no_virtual_size_is_sized_by_its_raw_size() {
        let base = 0x0040_0000u64;
        let mut text = section(".text", 0x1000, 0, false, true);
        text.raw_size = 0x2000;
        let module = ModuleLayout {
            base,
            name: "packed.exe".to_owned(),
            sections: vec![text],
        };
        let mappings = [map(base + 0x1000, 2 * PAGE, false, true)];
        let regions = build(&mappings, std::slice::from_ref(&module)).regions;
        assert_eq!(regions.len(), 2);
        assert_eq!((regions[1].start, regions[1].size), (base + 0x1000, 2 * PAGE));
        assert!(regions.iter().all(|r| r.owner != RegionOwner::Private));
    }

    /// Private memory lying between two sections of one module does not split
    /// the module into two rows.
    #[test]
    fn a_private_row_inside_a_module_does_not_split_its_group() {
        let base = 0x0040_0000u64;
        let module = ModuleLayout {
            base,
            name: "gap.dll".to_owned(),
            sections: vec![
                section(".text", 0x1000, 0x1000, false, true),
                section(".data", 0x4000, 0x1000, true, false),
            ],
        };
        // A mapped page in the unlabelled gap between .text and .data.
        let mappings = [map(base + 0x2000, PAGE, true, false)];
        let built = build(&mappings, std::slice::from_ref(&module));
        let groups = group(&built);
        let modules = groups
            .iter()
            .filter(|group| matches!(group, MapGroup::Module { .. }))
            .count();
        assert_eq!(modules, 1);
        assert!(groups.iter().any(|group| matches!(group, MapGroup::Region(_))));
    }

    /// A base so high that its header page would wrap is left out, not
    /// wrapped into a region with an underflowed size.
    #[test]
    fn a_module_at_the_top_of_the_address_space_is_skipped() {
        let module = ModuleLayout {
            base: 0xFFFF_FFFF_FFFF_F800,
            name: "wild.dll".to_owned(),
            sections: vec![section(".text", 0x1000, 0x1000, false, true)],
        };
        assert!(build(&[], std::slice::from_ref(&module)).regions.is_empty());
    }

    #[test]
    fn label_stacks_names_only_private_memory_inside_a_stack() {
        use rusty_box::cpu::LinearRange;
        let region = |start: u64, owner: RegionOwner| MapRegion {
            start,
            size: 0x1000,
            resident_pages: 1,
            total_pages: 1,
            access: None,
            owner,
            vad: None,
        };
        let mut map = MemoryMap {
            origin: MapOrigin::WindowsRecords,
            regions: vec![
                region(0x5C_0000, RegionOwner::Private),
                region(0x5D_0000, RegionOwner::Header { base: 0x5D_0000, module: "x.dll".to_owned() }),
                region(0x9_0000, RegionOwner::Private),
            ],
        };
        let threads = [WinThread {
            tid: 0x1A4,
            start: 0,
            teb: Some(0x7FF0_0000),
            stack: Some(LinearRange { start: 0x5B_0000, end: 0x60_0000 }),
            uses_fibers: false,
            saved: None,
        }];
        label_stacks(&mut map, &threads);
        // Private memory inside the stack is named for the thread.
        assert!(matches!(map.regions[0].owner, RegionOwner::Stack { tid: 0x1A4 }));
        // A module region inside the stack range keeps its own name.
        assert!(matches!(map.regions[1].owner, RegionOwner::Header { .. }));
        // Private memory outside every stack stays private.
        assert!(matches!(map.regions[2].owner, RegionOwner::Private));
    }
}
