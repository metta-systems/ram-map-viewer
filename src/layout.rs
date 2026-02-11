// Layout engine: converts a flat list of `MemoryRegion`s into visual blocks
// suitable for the wrapped linear map renderer.
//
// Key design: widths are computed directly in pixels for a given row width.
// Uses a log-compressed scale so that tiny regions (32 bytes) and huge regions
// (512 KiB) can coexist visually. Each row is stretched to fill the full
// available width — no dead space.

use crate::input::{MemoryRegion, MemoryType, RegionStatus};

/// A visual block to be rendered in the map.
#[derive(Debug, Clone)]
pub enum VisualBlock {
    /// A real memory region.
    Region {
        region: MemoryRegion,
        /// Width in pixels (set during row layout).
        px_width: f32,
    },
    /// A compressed gap marker.
    Gap {
        start: u64,
        end: u64,
        px_width: f32,
    },
}

impl VisualBlock {
    pub fn start(&self) -> u64 {
        match self {
            VisualBlock::Region { region, .. } => region.start,
            VisualBlock::Gap { start, .. } => *start,
        }
    }

    pub fn px_width(&self) -> f32 {
        match self {
            VisualBlock::Region { px_width, .. } => *px_width,
            VisualBlock::Gap { px_width, .. } => *px_width,
        }
    }

    pub fn set_px_width(&mut self, w: f32) {
        match self {
            VisualBlock::Region { px_width, .. } => *px_width = w,
            VisualBlock::Gap { px_width, .. } => *px_width = w,
        }
    }

    /// The "weight" used for proportional sizing.
    /// Uses log scale so 32 B and 512 KiB are both visible.
    fn weight(&self) -> f64 {
        match self {
            VisualBlock::Region { region, .. } => {
                let bytes = region.size().max(1) as f64;
                // log2(bytes) gives ~5 for 32B, ~19 for 512K — a 4:1 ratio
                // instead of 16000:1. We add a linear component to keep some
                // proportionality for similar-sized regions.
                let log_part = bytes.log2();
                let lin_part = bytes.sqrt();
                log_part + lin_part * 0.05
            }
            VisualBlock::Gap { .. } => {
                // Gaps get a fixed small weight.
                GAP_WEIGHT
            }
        }
    }
}

const GAP_WEIGHT: f64 = 6.0;

/// Layout configuration.
pub struct LayoutConfig {
    /// Gaps larger than this are compressed. Default: 64 KiB.
    pub gap_threshold: u64,
    /// Minimum pixel width for any block (so tiny regions remain clickable).
    pub min_block_px: f32,
    /// Target number of blocks per row (soft target for row wrapping).
    pub target_blocks_per_row: usize,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            gap_threshold: 64 * 1024,
            min_block_px: 24.0,
            target_blocks_per_row: 20,
        }
    }
}

/// A row of visual blocks with pixel widths that sum to exactly `row_px`.
#[derive(Debug, Clone)]
pub struct VisualRow {
    pub blocks: Vec<VisualBlock>,
}

/// Build the flat list of blocks (regions + gaps) from the sorted region list.
pub fn build_blocks(regions: &[MemoryRegion], config: &LayoutConfig) -> Vec<VisualBlock> {
    if regions.is_empty() {
        return Vec::new();
    }

    let mut entries: Vec<VisualBlock> = Vec::new();
    let mut frontier = regions[0].start;

    for region in regions {
        // Insert gap before this region if needed.
        if region.start > frontier {
            let gap_size = region.start - frontier;
            if gap_size > config.gap_threshold {
                entries.push(VisualBlock::Gap {
                    start: frontier,
                    end: region.start,
                    px_width: 0.0,
                });
            } else {
                // Small gap — show as a thin free region.
                entries.push(VisualBlock::Region {
                    region: MemoryRegion {
                        start: frontier,
                        end: region.start,
                        status: RegionStatus::Free,
                        mem_type: MemoryType::Cached,
                        permissions: String::new(),
                        name: "(gap)".into(),
                    },
                    px_width: 0.0,
                });
            }
        }

        // Compress large Free regions the same as gaps.
        let size = region.size();
        if region.status == RegionStatus::Free && size > config.gap_threshold {
            entries.push(VisualBlock::Gap {
                start: region.start,
                end: region.end,
                px_width: 0.0,
            });
        } else {
            entries.push(VisualBlock::Region {
                region: region.clone(),
                px_width: 0.0,
            });
        }

        if region.end > frontier {
            frontier = region.end;
        }
    }

    entries
}

/// Wrap blocks into rows and assign pixel widths.
///
/// Each row fills exactly `row_px` pixels. Blocks are distributed based on
/// their weight (log-compressed size), with a minimum pixel width enforced.
pub fn layout_rows(blocks: Vec<VisualBlock>, row_px: f32, config: &LayoutConfig) -> Vec<VisualRow> {
    if blocks.is_empty() {
        return Vec::new();
    }

    // Step 1: split blocks into row groups.
    // Strategy: greedily fill rows so that each has roughly
    // `target_blocks_per_row` blocks, but also ensure that the minimum
    // pixel width constraint is satisfiable (don't put so many blocks
    // that they can't all fit at min_block_px).
    let max_blocks_per_row = (row_px / config.min_block_px).floor() as usize;
    let target = config.target_blocks_per_row.min(max_blocks_per_row).max(1);

    let mut row_groups: Vec<Vec<VisualBlock>> = Vec::new();
    let mut current: Vec<VisualBlock> = Vec::new();

    for block in blocks {
        current.push(block);
        if current.len() >= target {
            row_groups.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        row_groups.push(current);
    }

    // Step 2: for each row, assign pixel widths proportional to weight,
    // then enforce min_block_px and redistribute.
    let mut rows: Vec<VisualRow> = Vec::new();

    for mut group in row_groups {
        assign_pixel_widths(&mut group, row_px, config.min_block_px);
        rows.push(VisualRow { blocks: group });
    }

    rows
}

/// Assign pixel widths to blocks in a single row, filling exactly `row_px`.
fn assign_pixel_widths(blocks: &mut [VisualBlock], row_px: f32, min_px: f32) {
    let n = blocks.len();
    if n == 0 {
        return;
    }

    // If every block at min_px already exceeds row_px, just distribute evenly.
    if n as f32 * min_px > row_px {
        let each = row_px / n as f32;
        for b in blocks.iter_mut() {
            b.set_px_width(each);
        }
        return;
    }

    // Compute weights.
    let weights: Vec<f64> = blocks.iter().map(|b| b.weight()).collect();
    let total_weight: f64 = weights.iter().sum();

    // First pass: proportional allocation.
    let mut widths: Vec<f32> = weights
        .iter()
        .map(|w| ((w / total_weight) * row_px as f64) as f32)
        .collect();

    // Second pass: enforce minimum, then redistribute the excess from
    // blocks that had to be enlarged.
    let mut deficit = 0.0f32;
    let mut flexible_weight = 0.0f64;

    for i in 0..n {
        if widths[i] < min_px {
            deficit += min_px - widths[i];
            widths[i] = min_px;
        } else {
            flexible_weight += weights[i];
        }
    }

    // Subtract deficit proportionally from flexible blocks.
    if deficit > 0.0 && flexible_weight > 0.0 {
        for i in 0..n {
            if widths[i] > min_px {
                let share = (weights[i] / flexible_weight) as f32 * deficit;
                widths[i] = (widths[i] - share).max(min_px);
            }
        }
    }

    // Final pass: correct any floating point drift so sum == row_px exactly.
    let sum: f32 = widths.iter().sum();
    let correction = row_px - sum;
    // Apply correction to the widest block.
    if let Some(widest_idx) = widths
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
        .map(|(i, _)| i)
    {
        widths[widest_idx] += correction;
    }

    // Apply.
    for (b, w) in blocks.iter_mut().zip(widths.iter()) {
        b.set_px_width(*w);
    }
}
