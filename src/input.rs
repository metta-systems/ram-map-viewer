// Input module: abstracts the source of memory region data.
//
// Replace `FileSource` with your own `MemorySource` implementation
// to feed live data instead of reading from a text file.

use std::path::Path;

/// Status of a memory region.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RegionStatus {
    Free,
    Used,
    Drop,
}

/// Cache / memory type attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemoryType {
    Cached,
    Device,
}

/// A single memory region entry.
#[derive(Debug, Clone)]
pub struct MemoryRegion {
    pub start: u64,
    pub end: u64,
    pub status: RegionStatus,
    pub mem_type: MemoryType,
    pub permissions: String,
    pub name: String,
}

impl MemoryRegion {
    pub fn size(&self) -> u64 {
        self.end.saturating_sub(self.start)
    }
}

/// Trait for providing memory region data.
/// Implement this to plug in live data sources.
pub trait MemorySource {
    fn load(&mut self) -> Result<Vec<MemoryRegion>, String>;
}

/// Reads memory regions from a text file.
pub struct FileSource {
    path: String,
}

impl FileSource {
    pub fn new(path: impl Into<String>) -> Self {
        Self { path: path.into() }
    }
}

impl MemorySource for FileSource {
    fn load(&mut self) -> Result<Vec<MemoryRegion>, String> {
        let content = std::fs::read_to_string(Path::new(&self.path)).map_err(|e| format!("{e}"))?;
        parse_regions(&content)
    }
}

/// Parse the text format into memory regions.
///
/// Expected line format:
/// ```text
///   [pa00_0020_0000 - pa00_0020_7260) |  29 KiB | (Free) C   RW PXN | RAM
/// ```
pub fn parse_regions(input: &str) -> Result<Vec<MemoryRegion>, String> {
    let mut regions = Vec::new();

    for (line_no, line) in input.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let region = parse_line(line).map_err(|e| format!("line {}: {e}: {line}", line_no + 1))?;
        regions.push(region);
    }

    // Sort by start address, then by end address (for overlapping regions).
    regions.sort_by_key(|r| (r.start, r.end));
    Ok(regions)
}

fn parse_line(line: &str) -> Result<MemoryRegion, String> {
    // Split on '|' to get: address range, size, attributes, name
    let parts: Vec<&str> = line.split('|').collect();
    if parts.len() < 4 {
        return Err("expected at least 4 '|'-separated fields".into());
    }

    let addr_part = parts[0].trim();
    // Size part (parts[1]) is informational — we compute from addresses.
    let attr_part = parts[2].trim();
    let name_part = parts[3].trim();

    // Parse addresses: [pa00_0020_0000 - pa00_0020_7260)
    let (start, end) = parse_address_range(addr_part)?;

    // Parse attributes: (Free) C   RW PXN
    let (status, mem_type, permissions) = parse_attributes(attr_part)?;

    Ok(MemoryRegion {
        start,
        end,
        status,
        mem_type,
        permissions,
        name: name_part.to_string(),
    })
}

fn parse_address_range(s: &str) -> Result<(u64, u64), String> {
    // Strip [ and )
    let s = s.trim_start_matches('[').trim_end_matches(')');
    let mut parts = s.split('-');
    let start_str = parts.next().ok_or("missing start address")?.trim();
    let end_str = parts.next().ok_or("missing end address")?.trim();

    let start = parse_pa_address(start_str)?;
    let end = parse_pa_address(end_str)?;
    Ok((start, end))
}

fn parse_pa_address(s: &str) -> Result<u64, String> {
    // Strip "pa" prefix, remove underscores, parse as hex.
    let hex_str: String = s
        .trim_start_matches("pa")
        .chars()
        .filter(|c| *c != '_')
        .collect();
    u64::from_str_radix(&hex_str, 16).map_err(|e| format!("bad address '{s}': {e}"))
}

fn parse_attributes(s: &str) -> Result<(RegionStatus, MemoryType, String), String> {
    // Format: (Free) C   RW PXN
    // Extract status in parens
    let paren_start = s.find('(').ok_or("missing '('")?;
    let paren_end = s.find(')').ok_or("missing ')'")?;
    let status_str = &s[paren_start + 1..paren_end];

    let status = match status_str {
        "Free" => RegionStatus::Free,
        "Used" => RegionStatus::Used,
        "Drop" => RegionStatus::Drop,
        other => return Err(format!("unknown status: {other}")),
    };

    let rest = s[paren_end + 1..].trim();
    // First token is memory type (C or Dev), rest is permissions.
    let mut tokens = rest.split_whitespace();
    let mem_type_str = tokens.next().unwrap_or("");
    let mem_type = match mem_type_str {
        "C" => MemoryType::Cached,
        "Dev" => MemoryType::Device,
        other => return Err(format!("unknown memory type: {other}")),
    };

    let permissions: String = tokens.collect::<Vec<_>>().join(" ");

    Ok((status, mem_type, permissions))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_line() {
        let line = "[pa00_0040_0000 - pa00_0040_3800) |  14 KiB | (Used) C   RO PX  | Nucleus code";
        let r = parse_line(line).unwrap();
        assert_eq!(r.start, 0x00_0040_0000);
        assert_eq!(r.end, 0x00_0040_3800);
        assert_eq!(r.status, RegionStatus::Used);
        assert_eq!(r.mem_type, MemoryType::Cached);
        assert_eq!(r.name, "Nucleus code");
    }

    #[test]
    fn test_parse_device_line() {
        let line = "[pa00_7e00_b840 - pa00_7e00_b87c) |  60 B   | (Used) Dev RW PXN | mailbox";
        let r = parse_line(line).unwrap();
        assert_eq!(r.mem_type, MemoryType::Device);
        assert_eq!(r.name, "mailbox");
    }
}
