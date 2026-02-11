use {
    colors::*,
    eframe::egui,
    egui::{Color32, CornerRadius, PopupAnchor, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2},
    input::{FileSource, MemoryRegion, MemorySource, RegionStatus},
    layout::{LayoutConfig, VisualBlock, VisualRow},
};

mod colors;
mod input;
mod layout;

/// Application state.
struct RamMapApp {
    regions: Vec<MemoryRegion>,
    rows: Vec<VisualRow>,
    layout_config: LayoutConfig,
    error: Option<String>,
    /// Index of the hovered region (by original region index), if any.
    hovered_region: Option<usize>,
    /// Index of the selected region in the table.
    selected_region: Option<usize>,
    /// Whether to show Drop regions.
    show_drop: bool,
    /// Whether to show Free regions.
    show_free: bool,
    /// Gap threshold in KiB (UI control).
    gap_threshold_kib: u64,
    /// Whether layout needs recomputation.
    dirty: bool,
    /// File path to load.
    file_path: String,
    /// Cached available width for detecting resize.
    last_map_width: f32,
}

impl RamMapApp {
    fn new(file_path: String) -> Self {
        let mut app = Self {
            regions: Vec::new(),
            rows: Vec::new(),
            layout_config: LayoutConfig::default(),
            error: None,
            hovered_region: None,
            selected_region: None,
            show_drop: true,
            show_free: true,
            gap_threshold_kib: 64,
            dirty: true,
            file_path,
            last_map_width: 0.0,
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

    fn recompute_layout(&mut self, map_width: f32, available_height: f32) {
        self.layout_config.gap_threshold = self.gap_threshold_kib * 1024;

        // Filter regions based on visibility settings.
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

        let blocks = layout::build_blocks(&filtered, &self.layout_config);
        self.rows = layout::layout_rows(blocks, map_width, available_height, &self.layout_config);
        self.last_map_width = map_width;
        self.dirty = false;
    }

    /// Find the original region index for a given VisualBlock::Region.
    fn find_region_index(&self, region: &MemoryRegion) -> Option<usize> {
        self.regions
            .iter()
            .position(|r| r.start == region.start && r.end == region.end && r.name == region.name)
    }
}

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
            });
        });

        // Right panel: region table.
        egui::SidePanel::right("table_panel")
            .default_width(420.0)
            .min_width(320.0)
            .show(ctx, |ui| {
                self.draw_table(ui);
            });

        // Central panel: visual map.
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(ref err) = self.error {
                ui.colored_label(Color32::RED, format!("Error: {err}"));
                return;
            }

            let gutter_width = 130.0;
            let map_width = (ui.available_width() - gutter_width - 16.0).max(100.0);
            let available_height = ui.available_height();

            // Recompute if dirty or if width changed significantly.
            if self.dirty || (self.last_map_width - map_width).abs() > 2.0 {
                self.recompute_layout(map_width, available_height);
            }

            self.draw_map(ui, map_width, gutter_width);
        });
    }
}

impl RamMapApp {
    fn draw_map(&mut self, ui: &mut egui::Ui, map_width: f32, gutter_width: f32) {
        let row_spacing = 3.0;

        // Total height from layout-computed row heights.
        let total_rows_height: f32 = self.rows.iter().map(|r| r.height_px).sum::<f32>()
            + row_spacing * (self.rows.len().saturating_sub(1)) as f32;
        let legend_height = 40.0;

        let total_width = gutter_width + map_width;
        let total_height = total_rows_height + legend_height;

        // Clone rows to avoid borrow conflict.
        let rows = self.rows.clone();

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // Allocate a single painter for the entire map area.
                let (response, painter) = ui.allocate_painter(
                    Vec2::new(total_width, total_height - legend_height),
                    Sense::hover(),
                );
                let origin = response.rect.min;
                let pointer = ui.input(|i| i.pointer.hover_pos());

                let mut hovered = None;
                let mut row_y = origin.y;

                for row in &rows {
                    if row.blocks.is_empty() {
                        continue;
                    }

                    let row_height = row.height_px;
                    let blocks_x = origin.x + gutter_width;

                    // --- Address gutter ---
                    let row_start_addr = row.blocks.first().map(|b| b.start()).unwrap_or(0);
                    let addr_text = format_addr(row_start_addr);
                    let gutter_center =
                        Pos2::new(origin.x + gutter_width - 8.0, row_y + row_height / 2.0);
                    painter.text(
                        gutter_center,
                        egui::Align2::RIGHT_CENTER,
                        &addr_text,
                        egui::FontId::monospace(11.0),
                        Color32::from_rgb(180, 180, 190),
                    );

                    // --- Blocks ---
                    let mut x = 0.0_f32;
                    for block in &row.blocks {
                        let w = block.px_width();
                        let rect = Rect::from_min_size(
                            Pos2::new(blocks_x + x, row_y),
                            Vec2::new(w, row_height),
                        );

                        match block {
                            VisualBlock::Region { region, .. } => {
                                let bg = region_color(region);
                                let is_hovered = self
                                    .hovered_region
                                    .and_then(|idx| self.regions.get(idx))
                                    .map(|hr| {
                                        hr.start == region.start
                                            && hr.end == region.end
                                            && hr.name == region.name
                                    })
                                    .unwrap_or(false);
                                let is_selected = self
                                    .selected_region
                                    .and_then(|idx| self.regions.get(idx))
                                    .map(|sr| {
                                        sr.start == region.start
                                            && sr.end == region.end
                                            && sr.name == region.name
                                    })
                                    .unwrap_or(false);

                                // Draw filled rect.
                                painter.rect_filled(rect, CornerRadius::same(2), bg);

                                // Highlight border.
                                if is_selected {
                                    painter.rect_stroke(
                                        rect,
                                        CornerRadius::same(2),
                                        Stroke::new(2.0, Color32::WHITE),
                                        StrokeKind::Outside,
                                    );
                                } else if is_hovered {
                                    painter.rect_stroke(
                                        rect,
                                        CornerRadius::same(2),
                                        Stroke::new(
                                            1.0,
                                            Color32::from_rgba_premultiplied(255, 255, 255, 80),
                                        ),
                                        StrokeKind::Outside,
                                    );
                                }

                                // Label: horizontal if wide enough, vertical if tall enough.
                                let text_color = label_color_for_bg(bg);
                                if w > 50.0 {
                                    // Horizontal label.
                                    let max_chars = ((w - 8.0) / 7.0) as usize;
                                    let label = truncate_label(&region.name, max_chars.max(3));
                                    painter.text(
                                        rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        label,
                                        egui::FontId::proportional(11.0),
                                        text_color,
                                    );
                                    // Size label below name if there's room.
                                    if row_height > 36.0 && w > 60.0 {
                                        let size_str = format_size(region.size());
                                        painter.text(
                                            Pos2::new(rect.center().x, rect.center().y + 13.0),
                                            egui::Align2::CENTER_CENTER,
                                            size_str,
                                            egui::FontId::proportional(9.0),
                                            Color32::from_rgba_premultiplied(
                                                text_color.r(),
                                                text_color.g(),
                                                text_color.b(),
                                                160,
                                            ),
                                        );
                                    }
                                } else if row_height > 50.0 && w > 14.0 {
                                    // Vertical label for narrow-but-tall blocks.
                                    let max_chars = ((row_height - 8.0) / 11.0) as usize;
                                    let label = truncate_label(&region.name, max_chars.max(2));
                                    let char_h = 11.0;
                                    let chars: Vec<char> = label.chars().collect();
                                    let total_h = chars.len() as f32 * char_h;
                                    let start_y = rect.center().y - total_h / 2.0;

                                    for (ci, ch) in chars.iter().enumerate() {
                                        painter.text(
                                            Pos2::new(
                                                rect.center().x,
                                                start_y + ci as f32 * char_h + char_h / 2.0,
                                            ),
                                            egui::Align2::CENTER_CENTER,
                                            ch.to_string(),
                                            egui::FontId::proportional(10.0),
                                            text_color,
                                        );
                                    }
                                }

                                // Hover detection.
                                if let Some(pos) = pointer
                                    && rect.contains(pos)
                                {
                                    hovered = self.find_region_index(region);
                                }
                            }
                            VisualBlock::Gap { start, end, .. } => {
                                // Dark rect with diagonal stripes.
                                painter.rect_filled(rect, CornerRadius::same(2), gap_color());

                                let stripe_spacing = 8.0;
                                let mut sx = rect.min.x - row_height;
                                while sx < rect.max.x + row_height {
                                    let p1 = Pos2::new(sx, rect.max.y);
                                    let p2 = Pos2::new(sx + row_height, rect.min.y);
                                    painter.line_segment(
                                        [
                                            Pos2::new(p1.x.max(rect.min.x), p1.y.min(rect.max.y)),
                                            Pos2::new(p2.x.min(rect.max.x), p2.y.max(rect.min.y)),
                                        ],
                                        Stroke::new(1.0, Color32::from_rgb(60, 60, 70)),
                                    );
                                    sx += stripe_spacing;
                                }

                                // Size label.
                                if w > 30.0 {
                                    let gap_size = end - start;
                                    let label = format_size(gap_size);
                                    painter.text(
                                        rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        label,
                                        egui::FontId::proportional(9.0),
                                        Color32::from_rgb(120, 120, 130),
                                    );
                                }
                            }
                        }

                        x += w;
                    }

                    row_y += row_height + row_spacing;
                }

                self.hovered_region = hovered;

                // Tooltip for hovered region.
                if let Some(idx) = self.hovered_region
                    && let Some(region) = self.regions.get(idx)
                {
                    egui::containers::Tooltip::always_open(
                        ui.ctx().clone(),
                        ui.layer_id(),
                        egui::Id::new("region_tooltip"),
                        PopupAnchor::Pointer,
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
                                input::MemoryType::Cached => "Cached",
                                input::MemoryType::Device => "Device",
                            },
                            region.permissions,
                        ));
                    });
                }

                // Legend at the bottom.
                ui.add_space(8.0);
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
                    legend_swatch(ui, gap_color(), "Gap");
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
                        // Header.
                        ui.strong("");
                        ui.strong("Name");
                        ui.strong("Start");
                        ui.strong("Size");
                        ui.strong("Status");
                        ui.end_row();

                        for (i, region) in self.regions.iter().enumerate() {
                            let is_hovered = self.hovered_region == Some(i);
                            let is_selected = self.selected_region == Some(i);

                            // Color swatch.
                            let (rect, _) = ui
                                .allocate_exact_size(Vec2::new(12.0, text_height), Sense::hover());
                            ui.painter().rect_filled(
                                rect,
                                CornerRadius::same(2),
                                region_color(region),
                            );

                            // Name (clickable).
                            let name_response =
                                ui.selectable_label(is_selected || is_hovered, &region.name);
                            if name_response.clicked() {
                                new_selected = if is_selected { None } else { Some(i) };
                            }
                            if name_response.hovered() {
                                self.hovered_region = Some(i);
                            }

                            // Start address.
                            ui.monospace(format_addr(region.start));

                            // Size.
                            ui.label(format_size(region.size()));

                            // Status badge.
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

fn main() -> eframe::Result<()> {
    let file_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "sample.txt".to_string());

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 720.0])
            .with_title("RAM Map Viewer"),
        ..Default::default()
    };

    eframe::run_native(
        "RAM Map Viewer",
        options,
        Box::new(move |_cc| Ok(Box::new(RamMapApp::new(file_path)))),
    )
}
