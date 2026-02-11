use {
    colors::*,
    eframe::egui,
    egui::{Color32, CornerRadius, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2},
    input::{FileSource, MemoryRegion, MemorySource, MemoryType, RegionStatus},
    layout::LayoutConfig,
};

mod colors;
mod hilbert;
mod input;
mod layout;

// ── Hilbert curve layout types ───────────────────────────────────────────────

/// A cell in the Hilbert curve grid, representing one "quantum" of the
/// linearized (gap-compressed) address space.
#[derive(Clone)]
struct HilbertCell {
    /// Index into the filtered region list.
    region_idx: usize,
    /// Whether this cell is a gap marker.
    is_gap: bool,
}

/// Pre-computed Hilbert map: each cell in the NxN grid maps to a region.
struct HilbertMap {
    order: u32,
    /// Grid side length = 2^order.
    grid_size: u32,
    /// Total cells = grid_size^2.
    cells: Vec<Option<HilbertCell>>,
    /// The filtered regions used to build this map.
    filtered_regions: Vec<MemoryRegion>,
    /// For each filtered region, how many cells it occupies.
    region_cell_counts: Vec<u32>,
}

// ── Application state ────────────────────────────────────────────────────────

struct RamMapApp {
    regions: Vec<MemoryRegion>,
    hilbert_map: Option<HilbertMap>,
    layout_config: LayoutConfig,
    error: Option<String>,
    hovered_region: Option<usize>,
    selected_region: Option<usize>,
    show_drop: bool,
    show_free: bool,
    gap_threshold_kib: u64,
    /// Hilbert curve order (grid side = 2^order). Higher = more detail.
    hilbert_order: u32,
    dirty: bool,
    file_path: String,
}

impl RamMapApp {
    fn new(file_path: String) -> Self {
        let mut app = Self {
            regions: Vec::new(),
            hilbert_map: None,
            layout_config: LayoutConfig::default(),
            error: None,
            hovered_region: None,
            selected_region: None,
            show_drop: true,
            show_free: true,
            gap_threshold_kib: 64,
            hilbert_order: 7, // 128x128 grid = 16384 cells
            dirty: true,
            file_path,
        };
        app.reload();
        app
    }

    fn reload(&mut self) {
        let mut source = FileSource::new(&self.file_path);
        match source.load() {
            Ok(regions) => {
                self.regions = regions;
                self.error = None;
                self.dirty = true;
            }
            Err(e) => {
                self.error = Some(e);
            }
        }
    }

    fn recompute_hilbert(&mut self) {
        self.layout_config.gap_threshold = self.gap_threshold_kib * 1024;

        let filtered: Vec<MemoryRegion> = self
            .regions
            .iter()
            .filter(|r| {
                if !self.show_drop && r.status == RegionStatus::Drop {
                    return false;
                }
                if !self.show_free && r.status == RegionStatus::Free {
                    return false;
                }
                true
            })
            .cloned()
            .collect();

        let order = self.hilbert_order;
        let grid_size = 1u32 << order;
        let total_cells = grid_size * grid_size;

        // Build the visual blocks (with gap compression) to know how to
        // distribute cells.
        let blocks = layout::compute_layout(&filtered, &mut self.layout_config);

        // Compute total display weight to distribute cells proportionally.
        let total_weight: f64 = blocks.iter().map(|b| b.display_width()).sum();

        if total_weight <= 0.0 || blocks.is_empty() {
            self.hilbert_map = Some(HilbertMap {
                order,
                grid_size,
                cells: vec![None; total_cells as usize],
                filtered_regions: filtered,
                region_cell_counts: Vec::new(),
            });
            self.dirty = false;
            return;
        }

        // For each block, compute how many Hilbert cells it gets.
        // Minimum 1 cell per block so nothing disappears.
        struct BlockAlloc {
            region_idx: usize,
            is_gap: bool,
            cell_count: u32,
        }

        let mut allocs: Vec<BlockAlloc> = Vec::new();
        let mut cells_used = 0u32;

        for block in &blocks {
            match block {
                layout::VisualBlock::Region {
                    region,
                    display_width,
                } => {
                    let idx = filtered
                        .iter()
                        .position(|r| {
                            r.start == region.start && r.end == region.end && r.name == region.name
                        })
                        .unwrap_or(0);

                    let raw_cells =
                        ((*display_width / total_weight) * total_cells as f64).round() as u32;
                    let cell_count = raw_cells.max(1);
                    allocs.push(BlockAlloc {
                        region_idx: idx,
                        is_gap: false,
                        cell_count,
                    });
                    cells_used += cell_count;
                }
                layout::VisualBlock::Gap { display_width, .. } => {
                    let raw_cells =
                        ((*display_width / total_weight) * total_cells as f64).round() as u32;
                    let cell_count = raw_cells.max(1);
                    allocs.push(BlockAlloc {
                        region_idx: 0,
                        is_gap: true,
                        cell_count,
                    });
                    cells_used += cell_count;
                }
            }
        }

        // Adjust to exactly fill total_cells.
        if cells_used > total_cells {
            if let Some(largest) = allocs.iter_mut().max_by_key(|a| a.cell_count) {
                let excess = cells_used - total_cells;
                if largest.cell_count > excess {
                    largest.cell_count -= excess;
                }
            }
        } else if cells_used < total_cells {
            if let Some(largest) = allocs.iter_mut().max_by_key(|a| a.cell_count) {
                largest.cell_count += total_cells - cells_used;
            }
        }

        // Fill cells array along the Hilbert curve.
        let mut cells: Vec<Option<HilbertCell>> = vec![None; total_cells as usize];
        let mut d = 0u32;
        let mut region_cell_counts = vec![0u32; filtered.len()];

        for alloc in &allocs {
            for _ in 0..alloc.cell_count {
                if d >= total_cells {
                    break;
                }
                cells[d as usize] = Some(HilbertCell {
                    region_idx: alloc.region_idx,
                    is_gap: alloc.is_gap,
                });
                if !alloc.is_gap && alloc.region_idx < region_cell_counts.len() {
                    region_cell_counts[alloc.region_idx] += 1;
                }
                d += 1;
            }
        }

        self.hilbert_map = Some(HilbertMap {
            order,
            grid_size,
            cells,
            filtered_regions: filtered,
            region_cell_counts,
        });
        self.dirty = false;
    }

    /// Find the original (unfiltered) region index for a filtered region.
    fn filtered_to_original(&self, hilbert: &HilbertMap, filtered_idx: usize) -> Option<usize> {
        let fr = hilbert.filtered_regions.get(filtered_idx)?;
        self.regions
            .iter()
            .position(|r| r.start == fr.start && r.end == fr.end && r.name == fr.name)
    }
}

// ── egui App ─────────────────────────────────────────────────────────────────

impl eframe::App for RamMapApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Top toolbar.
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("⟳ Reload").clicked() {
                    self.reload();
                }
                ui.separator();
                if ui.checkbox(&mut self.show_free, "Free").changed() {
                    self.dirty = true;
                }
                if ui.checkbox(&mut self.show_drop, "Drop").changed() {
                    self.dirty = true;
                }
                ui.separator();
                ui.label("Gap threshold:");
                let mut kib = self.gap_threshold_kib as f32;
                let slider = egui::Slider::new(&mut kib, 4.0..=4096.0)
                    .suffix(" KiB")
                    .logarithmic(true);
                if ui.add(slider).changed() {
                    self.gap_threshold_kib = kib as u64;
                    self.dirty = true;
                }
                ui.separator();
                ui.label("Detail:");
                let mut order = self.hilbert_order as f32;
                let order_slider = egui::Slider::new(&mut order, 4.0..=9.0)
                    .step_by(1.0)
                    .custom_formatter(|v, _| {
                        let n = 1u32 << (v as u32);
                        format!("{n}×{n}")
                    });
                if ui.add(order_slider).changed() {
                    self.hilbert_order = order as u32;
                    self.dirty = true;
                }
            });
        });

        // Right panel: region table (unchanged).
        egui::SidePanel::right("table_panel")
            .default_width(420.0)
            .min_width(320.0)
            .show(ctx, |ui| {
                self.draw_table(ui);
            });

        // Central panel: Hilbert curve map.
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(ref err) = self.error {
                ui.colored_label(Color32::RED, format!("Error: {err}"));
                return;
            }

            if self.dirty {
                self.recompute_hilbert();
            }

            self.draw_hilbert_map(ui);
        });
    }
}

// ── Drawing ──────────────────────────────────────────────────────────────────

impl RamMapApp {
    fn draw_hilbert_map(&mut self, ui: &mut egui::Ui) {
        let hilbert = match &self.hilbert_map {
            Some(h) => h,
            None => return,
        };

        // Extract everything we need from hilbert into locals,
        // so we don't hold a borrow on self inside the closure.
        let grid_size = hilbert.grid_size;
        let order = hilbert.order;
        let cells = &hilbert.cells;
        let filtered_regions = &hilbert.filtered_regions;
        let region_cell_counts = &hilbert.region_cell_counts;

        // Pre-build a lookup table: filtered_idx → original region index.
        let filtered_to_original: Vec<Option<usize>> = filtered_regions
            .iter()
            .map(|fr| {
                self.regions
                    .iter()
                    .position(|r| r.start == fr.start && r.end == fr.end && r.name == fr.name)
            })
            .collect();

        // Determine available square size for the map.
        let avail = ui.available_size();
        let map_side = (avail.x.min(avail.y) - 20.0).max(100.0);
        let cell_px = map_side / grid_size as f32;

        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let desired_size = Vec2::splat(map_side);
                let (response, painter) = ui.allocate_painter(desired_size, Sense::hover());
                let origin = response.rect.min;

                let mut hovered: Option<usize> = None;
                let pointer_pos = ui.input(|i| i.pointer.hover_pos());

                // Paint each cell.
                for d in 0..(grid_size * grid_size) {
                    let cell = match &cells[d as usize] {
                        Some(c) => c,
                        None => continue,
                    };

                    let (hx, hy) = hilbert::d2xy(order, d);
                    let rect = Rect::from_min_size(
                        Pos2::new(
                            origin.x + hx as f32 * cell_px,
                            origin.y + hy as f32 * cell_px,
                        ),
                        Vec2::splat(cell_px),
                    );

                    let color = if cell.is_gap {
                        gap_color()
                    } else {
                        region_color(&filtered_regions[cell.region_idx])
                    };

                    let original_idx = if !cell.is_gap {
                        filtered_to_original[cell.region_idx]
                    } else {
                        None
                    };

                    let is_selected =
                        original_idx.is_some() && self.selected_region == original_idx;
                    let is_hovered_region =
                        original_idx.is_some() && self.hovered_region == original_idx;

                    let final_color = if is_selected {
                        brighten(color, 0.5)
                    } else if is_hovered_region {
                        brighten(color, 0.25)
                    } else {
                        color
                    };

                    painter.rect_filled(rect, CornerRadius::ZERO, final_color);

                    if is_selected {
                        painter.rect_stroke(
                            rect,
                            CornerRadius::ZERO,
                            Stroke::new(1.0, Color32::WHITE),
                            StrokeKind::Inside,
                        );
                    }

                    if let Some(pos) = pointer_pos
                        && rect.contains(pos)
                        && !cell.is_gap
                    {
                        hovered = original_idx;
                    }
                }

                // Region boundary outlines.
                if cell_px >= 2.0 {
                    let boundary_stroke =
                        Stroke::new((cell_px * 0.12).clamp(0.5, 1.5), Color32::from_gray(30));

                    for d in 0..(grid_size * grid_size) {
                        let cell = match &cells[d as usize] {
                            Some(c) => c,
                            None => continue,
                        };
                        let (hx, hy) = hilbert::d2xy(order, d);
                        let cell_key = (cell.region_idx, cell.is_gap);

                        // Right neighbor.
                        if hx + 1 < grid_size {
                            let nd = hilbert::xy2d(order, hx + 1, hy);
                            if let Some(nc) = &cells[nd as usize]
                                && (nc.region_idx, nc.is_gap) != cell_key
                            {
                                let x = origin.x + (hx + 1) as f32 * cell_px;
                                let y_top = origin.y + hy as f32 * cell_px;
                                painter.line_segment(
                                    [Pos2::new(x, y_top), Pos2::new(x, y_top + cell_px)],
                                    boundary_stroke,
                                );
                            }
                        }
                        // Bottom neighbor.
                        if hy + 1 < grid_size {
                            let nd = hilbert::xy2d(order, hx, hy + 1);
                            if let Some(nc) = &cells[nd as usize]
                                && (nc.region_idx, nc.is_gap) != cell_key
                            {
                                let y = origin.y + (hy + 1) as f32 * cell_px;
                                let x_left = origin.x + hx as f32 * cell_px;
                                painter.line_segment(
                                    [Pos2::new(x_left, y), Pos2::new(x_left + cell_px, y)],
                                    boundary_stroke,
                                );
                            }
                        }
                    }
                }

                // Region name labels at centroids.
                if cell_px >= 3.0 {
                    for (fi, fr) in filtered_regions.iter().enumerate() {
                        let cell_count = region_cell_counts.get(fi).copied().unwrap_or(0);
                        if cell_count < 4 {
                            continue;
                        }

                        let mut sum_x = 0.0f64;
                        let mut sum_y = 0.0f64;
                        let mut count = 0u32;

                        for d in 0..(grid_size * grid_size) {
                            if let Some(c) = &cells[d as usize]
                                && !c.is_gap
                                && c.region_idx == fi
                            {
                                let (hx, hy) = hilbert::d2xy(order, d);
                                sum_x += hx as f64 + 0.5;
                                sum_y += hy as f64 + 0.5;
                                count += 1;
                            }
                        }

                        if count == 0 {
                            continue;
                        }

                        let cx = origin.x + (sum_x / count as f64) as f32 * cell_px;
                        let cy = origin.y + (sum_y / count as f64) as f32 * cell_px;

                        let approx_radius = (count as f32).sqrt() * cell_px * 0.5;
                        if approx_radius < 20.0 {
                            continue;
                        }

                        let max_chars = (approx_radius / 4.0) as usize;
                        let label = truncate_label(&fr.name, max_chars.max(4));
                        let color = region_color(fr);
                        let text_color = label_color_for_bg(color);

                        let galley = painter.layout_no_wrap(
                            label.clone(),
                            egui::FontId::proportional(10.0),
                            text_color,
                        );
                        let text_rect = Rect::from_center_size(
                            Pos2::new(cx, cy),
                            galley.size() + Vec2::splat(4.0),
                        );
                        painter.rect_filled(
                            text_rect,
                            CornerRadius::same(2),
                            color.gamma_multiply(0.85),
                        );
                        painter.text(
                            Pos2::new(cx, cy),
                            egui::Align2::CENTER_CENTER,
                            label,
                            egui::FontId::proportional(10.0),
                            text_color,
                        );
                    }
                }

                self.hovered_region = hovered;

                // Tooltip.
                if let Some(idx) = self.hovered_region
                    && let Some(region) = self.regions.get(idx)
                {
                    egui::containers::Tooltip::always_open(
                        ui.ctx().clone(),
                        ui.layer_id(),
                        egui::Id::new("region_tooltip"),
                        egui::PopupAnchor::Pointer,
                    )
                    .show(|ui| {
                        ui.strong(&region.name);
                        ui.monospace(format!(
                            "{} → {}",
                            format_addr(region.start),
                            format_addr(region.end)
                        ));
                        ui.label(format!(
                            "Size: {} | {} | {} | {}",
                            format_size(region.size()),
                            status_label(region.status),
                            match region.mem_type {
                                MemoryType::Cached => "Cached",
                                MemoryType::Device => "Device",
                            },
                            region.permissions,
                        ));
                    });
                }

                // Legend.
                ui.add_space(16.0);
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    legend_swatch(ui, Color32::from_rgb(70, 130, 220), "Code");
                    legend_swatch(ui, Color32::from_rgb(100, 180, 230), "RO Data");
                    legend_swatch(ui, Color32::from_rgb(80, 170, 80), "Data");
                    legend_swatch(ui, Color32::from_rgb(120, 190, 120), "BSS");
                    legend_swatch(ui, Color32::from_rgb(230, 160, 50), "Stack");
                    legend_swatch(ui, Color32::from_rgb(160, 120, 210), "Page Tables");
                    legend_swatch(ui, Color32::from_rgb(220, 80, 60), "Device/MMIO");
                    legend_swatch(ui, Color32::from_rgb(140, 100, 60), "Init/Boot");
                    legend_swatch(ui, Color32::from_rgb(170, 130, 90), "DTB");
                    legend_swatch(ui, Color32::from_rgb(60, 60, 70), "Free");
                    legend_swatch(ui, gap_color(), "Gap (compressed)");
                });
            });
    }

    fn draw_table(&mut self, ui: &mut egui::Ui) {
        ui.heading("Memory Regions");
        ui.separator();

        let text_height = 16.0;
        let mut new_selected = self.selected_region;

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new("region_table")
                    .striped(true)
                    .min_col_width(10.0)
                    .show(ui, |ui| {
                        ui.strong("");
                        ui.strong("Name");
                        ui.strong("Start");
                        ui.strong("Size");
                        ui.strong("Status");
                        ui.end_row();

                        for (i, region) in self.regions.iter().enumerate() {
                            let is_hovered = self.hovered_region == Some(i);
                            let is_selected = self.selected_region == Some(i);

                            let (rect, _) = ui
                                .allocate_exact_size(Vec2::new(12.0, text_height), Sense::hover());
                            ui.painter().rect_filled(
                                rect,
                                CornerRadius::same(2),
                                region_color(region),
                            );

                            let name_response =
                                ui.selectable_label(is_selected || is_hovered, &region.name);
                            if name_response.clicked() {
                                new_selected = if is_selected { None } else { Some(i) };
                            }
                            if name_response.hovered() {
                                self.hovered_region = Some(i);
                            }

                            ui.monospace(format_addr(region.start));
                            ui.label(format_size(region.size()));

                            let badge_color = status_color(region.status);
                            let badge_rect = ui
                                .allocate_exact_size(Vec2::new(36.0, text_height), Sense::hover())
                                .0;
                            ui.painter().rect_filled(
                                badge_rect,
                                CornerRadius::same(3),
                                badge_color,
                            );
                            ui.painter().text(
                                badge_rect.center(),
                                egui::Align2::CENTER_CENTER,
                                status_label(region.status),
                                egui::FontId::proportional(10.0),
                                Color32::WHITE,
                            );

                            ui.end_row();
                        }
                    });
            });

        self.selected_region = new_selected;
    }
}

// ── Utilities ────────────────────────────────────────────────────────────────

fn legend_swatch(ui: &mut egui::Ui, color: Color32, label: &str) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(12.0, 12.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(2), color);
    ui.label(label);
    ui.add_space(8.0);
}

fn truncate_label(s: &str, max_chars: usize) -> String {
    if s.len() <= max_chars {
        s.to_string()
    } else if max_chars > 3 {
        format!("{}…", &s[..max_chars - 1])
    } else {
        "…".to_string()
    }
}

/// Brighten a color by mixing towards white.
fn brighten(c: Color32, amount: f32) -> Color32 {
    let r = c.r() as f32 + (255.0 - c.r() as f32) * amount;
    let g = c.g() as f32 + (255.0 - c.g() as f32) * amount;
    let b = c.b() as f32 + (255.0 - c.b() as f32) * amount;
    Color32::from_rgb(r as u8, g as u8, b as u8)
}

fn main() -> eframe::Result<()> {
    let file_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "sample.txt".to_string());

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 720.0])
            .with_title("RAM Map Viewer — Hilbert"),
        ..Default::default()
    };

    eframe::run_native(
        "RAM Map Viewer — Hilbert",
        options,
        Box::new(move |_cc| Ok(Box::new(RamMapApp::new(file_path)))),
    )
}
