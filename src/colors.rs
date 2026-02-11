// Color scheme and formatting utilities for the RAM map viewer.

use {
    crate::input::{MemoryRegion, MemoryType, RegionStatus},
    egui::Color32,
};

/// Color palette for memory regions.
/// Colors are chosen for clear visual distinction and readability.
pub fn region_color(region: &MemoryRegion) -> Color32 {
    // First branch on memory type, then on status/name.
    match region.mem_type {
        MemoryType::Device => {
            // Device/MMIO regions: warm reds/oranges.
            match region.status {
                RegionStatus::Used => Color32::from_rgb(220, 80, 60), // red
                RegionStatus::Free => Color32::from_rgb(180, 120, 90), // muted orange
                RegionStatus::Drop => Color32::from_rgb(200, 100, 80), // dim red
            }
        }
        MemoryType::Cached => {
            // Cached RAM: branch by status and then by name patterns.
            match region.status {
                RegionStatus::Free => Color32::from_rgb(60, 60, 70), // dark gray
                RegionStatus::Drop => drop_color(region),
                RegionStatus::Used => used_color(region),
            }
        }
    }
}

fn used_color(region: &MemoryRegion) -> Color32 {
    let name = region.name.to_lowercase();
    if name.contains("code") {
        Color32::from_rgb(70, 130, 220) // blue - executable code
    } else if name.contains("read-only") || name.contains("rodata") {
        Color32::from_rgb(100, 180, 230) // light blue - read-only data
    } else if name.contains("bss") {
        Color32::from_rgb(120, 190, 120) // green - BSS
    } else if name.contains("data") {
        Color32::from_rgb(80, 170, 80) // green - data
    } else if name.contains("stack") {
        Color32::from_rgb(230, 160, 50) // orange - stacks
    } else if name.contains("mapping") || name.contains("l0") || name.contains("page") {
        Color32::from_rgb(160, 120, 210) // purple - page tables
    } else if name.contains("heap") {
        Color32::from_rgb(220, 200, 60) // yellow - heap
    } else {
        Color32::from_rgb(100, 160, 200) // default blue-gray
    }
}

fn drop_color(region: &MemoryRegion) -> Color32 {
    let name = region.name.to_lowercase();
    if name.contains("init") || name.contains("thread") {
        Color32::from_rgb(140, 100, 60) // brown - init/boot transient
    } else if name.contains("dtb") {
        Color32::from_rgb(170, 130, 90) // tan - DTB
    } else if name.contains("identity") {
        Color32::from_rgb(120, 90, 70) // dark brown
    } else {
        Color32::from_rgb(130, 110, 90) // muted brown
    }
}

/// Color for compressed gap markers.
pub fn gap_color() -> Color32 {
    Color32::from_rgb(40, 40, 50)
}

/// Text color for labels on a given background.
pub fn label_color_for_bg(bg: Color32) -> Color32 {
    let luminance = 0.299 * bg.r() as f32 + 0.587 * bg.g() as f32 + 0.114 * bg.b() as f32;
    if luminance > 140.0 {
        Color32::from_rgb(20, 20, 20)
    } else {
        Color32::from_rgb(230, 230, 230)
    }
}

/// Format a byte count into a human-readable string.
pub fn format_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.1} GiB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

/// Format an address as `0xNNNN_NNNN_NNNN`.
pub fn format_addr(addr: u64) -> String {
    let hex = format!("{addr:012x}");
    // Insert underscores every 4 chars: 0000_0040_0000
    let mut result = String::with_capacity(16);
    for (i, ch) in hex.chars().enumerate() {
        if i > 0 && i % 4 == 0 {
            result.push('_');
        }
        result.push(ch);
    }
    format!("0x{result}")
}

/// Status badge text.
pub fn status_label(s: RegionStatus) -> &'static str {
    match s {
        RegionStatus::Free => "Free",
        RegionStatus::Used => "Used",
        RegionStatus::Drop => "Drop",
    }
}

/// Status badge color.
pub fn status_color(s: RegionStatus) -> Color32 {
    match s {
        RegionStatus::Free => Color32::from_rgb(80, 80, 90),
        RegionStatus::Used => Color32::from_rgb(60, 160, 60),
        RegionStatus::Drop => Color32::from_rgb(180, 140, 50),
    }
}
