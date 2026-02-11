// Layout engine: converts a flat list of `MemoryRegion`s into visual blocks
// suitable for the wrapped linear map renderer.
//
// Handles:
// - Gap detection and compression (large free/empty spans → collapsed marker)
// - Merging overlapping regions into visual layers
// - Assigning display widths proportional to size (with min-width for tiny regions)

use crate::input::{MemoryRegion, MemoryType, RegionStatus};

/// A visual block to be rendered in the map.
#[derive(Debug, Clone)]
pub enum VisualBlock {
    /// A real memory region.
    Region {
        region: MemoryRegion,
        /// Display width in "layout units" (will be mapped to pixels).
        display_width: f64,
    },
    /// A compressed gap marker.
    Gap {
        start: u64,
        end: u64,
        display_width: f64,
    },
}

impl VisualBlock {
    pub fn start(&self) -> u64 {
        match self {
            VisualBlock::Region { region, .. } => region.start,
            VisualBlock::Gap { start, .. } => *start,
        }
    }

    pub fn end(&self) -> u64 {
        match self {
            VisualBlock::Region { region, .. } => region.end,
            VisualBlock::Gap { end, .. } => *end,
        }
    }

    pub fn display_width(&self) -> f64 {
        match self {
            VisualBlock::Region { display_width, .. } => *display_width,
            VisualBlock::Gap { display_width, .. } => *display_width,
        }
    }
}

/// Layout configuration.
pub struct LayoutConfig {
    /// Gaps larger than this are compressed. Default: 64 KiB.
    pub gap_threshold: u64,
    /// The fixed display width for a compressed gap (in layout units).
    pub gap_display_width: f64,
    /// Minimum display width for any region (so tiny regions remain visible).
    pub min_region_width: f64,
    /// Scale factor: layout units per byte for "normal" regions.
    /// This is computed dynamically.
    pub bytes_per_unit: f64,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            gap_threshold: 64 * 1024,
            gap_display_width: 30.0,
            min_region_width: 20.0,
            bytes_per_unit: 0.0, // computed
        }
    }
}

/// A row of visual blocks that fits within the available width.
#[derive(Debug, Clone)]
pub struct VisualRow {
    pub blocks: Vec<VisualBlock>,
    pub total_width: f64,
}

/// Compute the visual layout from a sorted list of memory regions.
pub fn compute_layout(regions: &[MemoryRegion], config: &mut LayoutConfig) -> Vec<VisualBlock> {
    if regions.is_empty() {
        return Vec::new();
    }

    // First pass: identify all blocks (regions + gaps), compute total "real" bytes
    // that will be displayed at linear scale.
    let mut entries: Vec<VisualBlock> = Vec::new();
    let mut total_real_bytes: u64 = 0;

    // Track the frontier of the address space we've covered.
    // Regions may overlap (same start address, different layers), so we
    // need to handle that.
    let mut frontier = regions[0].start;

    for region in regions {
        // Is there a gap before this region?
        if region.start > frontier {
            let gap_size = region.start - frontier;
            if gap_size > config.gap_threshold {
                // Compressed gap
                entries.push(VisualBlock::Gap {
                    start: frontier,
                    end: region.start,
                    display_width: config.gap_display_width,
                });
            } else {
                // Small gap — treat as a free region
                entries.push(VisualBlock::Region {
                    region: MemoryRegion {
                        start: frontier,
                        end: region.start,
                        status: RegionStatus::Free,
                        mem_type: MemoryType::Cached,
                        permissions: String::new(),
                        name: "(gap)".into(),
                    },
                    display_width: 0.0, // computed later
                });
                total_real_bytes += gap_size;
            }
        }

        // Check if this is a large Free region that should be compressed.
        let size = region.size();
        if region.status == RegionStatus::Free && size > config.gap_threshold {
            entries.push(VisualBlock::Gap {
                start: region.start,
                end: region.end,
                display_width: config.gap_display_width,
            });
        } else {
            entries.push(VisualBlock::Region {
                region: region.clone(),
                display_width: 0.0, // computed later
            });
            total_real_bytes += size;
        }

        if region.end > frontier {
            frontier = region.end;
        }
    }

    // Second pass: compute scale and display widths.
    // We want the total "real" content to fill ~1000 layout units,
    // so the user gets a reasonable visual density.
    let target_total_units = 1000.0;
    if total_real_bytes > 0 {
        config.bytes_per_unit = total_real_bytes as f64 / target_total_units;
    } else {
        config.bytes_per_unit = 1.0;
    }

    for block in &mut entries {
        match block {
            VisualBlock::Region {
                region,
                display_width,
            } => {
                let w = (region.size() as f64 / config.bytes_per_unit).max(config.min_region_width);
                *display_width = w;
            }
            VisualBlock::Gap { display_width, .. } => {
                // Already set to gap_display_width, but ensure it.
                *display_width = config.gap_display_width;
            }
        }
    }

    entries
}

/// Wrap visual blocks into rows that fit within `row_width` layout units.
pub fn wrap_into_rows(blocks: &[VisualBlock], row_width: f64) -> Vec<VisualRow> {
    let mut rows = Vec::new();
    let mut current_row = Vec::new();
    let mut current_width = 0.0;

    for block in blocks {
        let bw = block.display_width();

        // If adding this block would overflow, start a new row.
        // Exception: if the row is empty, always add (even if oversized).
        if !current_row.is_empty() && current_width + bw > row_width {
            rows.push(VisualRow {
                blocks: std::mem::take(&mut current_row),
                total_width: current_width,
            });
            current_width = 0.0;
        }

        current_width += bw;
        current_row.push(block.clone());
    }

    if !current_row.is_empty() {
        rows.push(VisualRow {
            blocks: current_row,
            total_width: current_width,
        });
    }

    rows
}
