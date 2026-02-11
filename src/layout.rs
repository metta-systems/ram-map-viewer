// Layout engine: converts a flat list of `MemoryRegion`s into visual blocks
// suitable for the wrapped linear map renderer.
//
// Key design: widths are computed directly in pixels for a given row width.
// Uses a log-compressed scale so that tiny regions (32 bytes) and huge regions
// (512 KiB) can coexist visually. Each row is stretched to fill the full
// available width — no dead space.

use crate::input::MemoryRegion;

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
    Gap { start: u64, end: u64, px_width: f32 },
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

    /// The label text this block would display.
    pub fn label_text(&self) -> String {
        match self {
            VisualBlock::Region { region, .. } => region.name.clone(),
            VisualBlock::Gap { start, end, .. } => format_size_compact(*end - *start),
        }
    }

    /// Minimum pixel width needed to display this block's label readably.
    /// Accounts for character count + padding.
    fn min_readable_px(&self) -> f32 {
        let label = self.label_text();
        let char_width = 7.0_f32; // approximate for proportional 11px font
        let padding = 16.0; // left + right padding
        let text_px = label.len() as f32 * char_width + padding;
        // Gaps can be narrower — they're less important.
        match self {
            VisualBlock::Gap { .. } => text_px.max(40.0),
            VisualBlock::Region { .. } => text_px.max(50.0),
        }
    }

    /// Size bonus: extra pixels awarded proportional to byte size.
    /// Larger regions get more visual weight beyond their label width.
    /// Returns a bonus in pixels (will be scaled to fit the row).
    fn size_bonus(&self) -> f32 {
        match self {
            VisualBlock::Region { region, .. } => {
                let bytes = region.size().max(1) as f64;
                // sqrt gives gentle scaling: 1 KiB → 32, 1 MiB → 1024, 1 GiB → 32K
                // We scale this down to a reasonable pixel bonus.
                (bytes.sqrt() * 0.1) as f32
            }
            VisualBlock::Gap { .. } => 0.0, // gaps don't get size bonus
        }
    }
}

/// Compact size formatting for gap labels.
fn format_size_compact(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1}M", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.0}K", bytes as f64 / 1024.0)
    } else {
        format!("{}B", bytes)
    }
}

/// Layout configuration.
#[derive(Default)]
pub struct LayoutConfig {}

/// A row of visual blocks with pixel widths that sum to exactly `row_px`.
#[derive(Debug, Clone)]
pub struct VisualRow {
    pub blocks: Vec<VisualBlock>,
    /// Address range this row covers (end of last block - start of first block).
    pub address_span: u64,
    /// Computed pixel height (proportional to address_span relative to other rows).
    pub height_px: f32,
}

/// Build the flat list of blocks (regions + gaps) from the sorted region list.
///
/// Holes in the address space (addresses not covered by any region) become
/// `VisualBlock::Gap` entries. Free regions are kept as normal region blocks.
pub fn build_blocks(regions: &[MemoryRegion], _config: &LayoutConfig) -> Vec<VisualBlock> {
    if regions.is_empty() {
        return Vec::new();
    }

    let mut entries: Vec<VisualBlock> = Vec::new();
    let mut frontier = regions[0].start;

    for region in regions {
        // Insert a gap block for any hole in the address space.
        if region.start > frontier {
            entries.push(VisualBlock::Gap {
                start: frontier,
                end: region.start,
                px_width: 0.0,
            });
        }

        entries.push(VisualBlock::Region {
            region: region.clone(),
            px_width: 0.0,
        });

        if region.end > frontier {
            frontier = region.end;
        }
    }

    entries
}

/// Wrap blocks into rows and assign pixel widths and heights.
///
/// **Text-driven layout**: each block needs enough width to show its label.
/// Rows are filled greedily — a block is added to the current row as long as
/// all blocks in the row can still fit at their minimum readable width.
/// After row assignment, surplus pixels are distributed proportionally to
/// each block's size bonus, so larger regions appear visually bigger.
///
/// **Address-proportional heights**: each row's height is proportional to the
/// address range it covers (using log scale to prevent one huge gap from
/// dominating). This makes rows spanning large address ranges visually taller.
pub fn layout_rows(
    blocks: Vec<VisualBlock>,
    row_px: f32,
    available_height: f32,
    _config: &LayoutConfig,
) -> Vec<VisualRow> {
    if blocks.is_empty() {
        return Vec::new();
    }

    // Step 1: greedily fill rows based on label widths.
    let mut row_groups: Vec<Vec<VisualBlock>> = Vec::new();
    let mut current: Vec<VisualBlock> = Vec::new();
    let mut current_min_total: f32 = 0.0;

    for block in blocks {
        let block_min = block.min_readable_px();

        // Would adding this block cause the row to exceed row_px
        // when every block is at its minimum readable width?
        if !current.is_empty() && current_min_total + block_min > row_px {
            // Flush current row.
            row_groups.push(std::mem::take(&mut current));
            current_min_total = 0.0;
        }

        current_min_total += block_min;
        current.push(block);
    }
    if !current.is_empty() {
        row_groups.push(current);
    }

    // Step 2: assign pixel widths per row.
    let mut rows: Vec<VisualRow> = Vec::new();

    for mut group in row_groups {
        assign_text_driven_widths(&mut group, row_px);

        // Compute address span for this row.
        let row_start = group.first().map(|b| b.start()).unwrap_or(0);
        let row_end = group.last().map(|b| b.end()).unwrap_or(0);
        let address_span = row_end.saturating_sub(row_start).max(1);

        rows.push(VisualRow {
            blocks: group,
            address_span,
            height_px: 0.0, // computed below
        });
    }

    // Step 3: compute row heights proportional to address span (log scale).
    // Using log prevents a single row with a huge gap from eating all vertical space.
    let min_row_height = 28.0_f32;
    let max_row_height = available_height * 0.6; // no single row takes >60%
    let row_spacing = 3.0;
    let total_spacing = row_spacing * (rows.len().saturating_sub(1)) as f32;
    let usable_height = (available_height - total_spacing).max(rows.len() as f32 * min_row_height);

    let log_spans: Vec<f64> = rows
        .iter()
        .map(|r| (r.address_span as f64).ln_1p()) // ln(1 + span) for smooth scaling
        .collect();
    let total_log: f64 = log_spans.iter().sum();

    if total_log > 0.0 {
        // First pass: proportional heights.
        let mut heights: Vec<f32> = log_spans
            .iter()
            .map(|ls| ((ls / total_log) * usable_height as f64) as f32)
            .collect();

        // Second pass: enforce min/max, redistribute.
        let mut clamped_total = 0.0_f32;
        let mut flexible_log = 0.0_f64;
        for (i, h) in heights.iter_mut().enumerate() {
            if *h < min_row_height {
                clamped_total += min_row_height - *h;
                *h = min_row_height;
            } else if *h > max_row_height {
                clamped_total -= *h - max_row_height;
                *h = max_row_height;
            } else {
                flexible_log += log_spans[i];
            }
        }
        // Redistribute clamping debt/surplus among flexible rows.
        if clamped_total.abs() > 0.1 && flexible_log > 0.0 {
            for (i, h) in heights.iter_mut().enumerate() {
                if *h > min_row_height && *h < max_row_height {
                    let share = (log_spans[i] / flexible_log) as f32 * clamped_total;
                    *h = (*h - share).clamp(min_row_height, max_row_height);
                }
            }
        }

        // Final correction for fp drift.
        let sum: f32 = heights.iter().sum();
        let correction = usable_height - sum;
        if let Some(tallest) = heights
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(i, _)| i)
        {
            heights[tallest] = (heights[tallest] + correction).max(min_row_height);
        }

        for (row, h) in rows.iter_mut().zip(heights.iter()) {
            row.height_px = *h;
        }
    } else {
        // Fallback: equal heights.
        let h = usable_height / rows.len() as f32;
        for row in &mut rows {
            row.height_px = h;
        }
    }

    rows
}

/// Assign pixel widths to blocks in a row using text-driven sizing.
///
/// Each block starts at its minimum readable width. The remaining pixels
/// (surplus) are distributed proportionally to each block's size_bonus,
/// so that larger regions visually expand while all labels remain readable.
fn assign_text_driven_widths(blocks: &mut [VisualBlock], row_px: f32) {
    let n = blocks.len();
    if n == 0 {
        return;
    }

    // Compute minimums.
    let min_widths: Vec<f32> = blocks.iter().map(|b| b.min_readable_px()).collect();
    let total_min: f32 = min_widths.iter().sum();

    // If minimums already exceed row_px, scale everything down proportionally.
    if total_min >= row_px {
        let scale = row_px / total_min;
        for (b, &mw) in blocks.iter_mut().zip(min_widths.iter()) {
            b.set_px_width(mw * scale);
        }
        return;
    }

    // Surplus pixels to distribute.
    let surplus = row_px - total_min;

    // Distribute surplus by size_bonus.
    let bonuses: Vec<f32> = blocks.iter().map(|b| b.size_bonus()).collect();
    let total_bonus: f32 = bonuses.iter().sum();

    let mut widths = min_widths;

    if total_bonus > 0.0 {
        for i in 0..n {
            widths[i] += surplus * (bonuses[i] / total_bonus);
        }
    } else {
        // No size bonus at all (e.g. row of only gaps) — distribute evenly.
        let each = surplus / n as f32;
        for w in &mut widths {
            *w += each;
        }
    }

    // Correct floating point drift.
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
