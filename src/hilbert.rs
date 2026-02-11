// Hilbert curve mapping between 1D indices and 2D coordinates.
//
// Uses the standard algorithm for mapping between a 1D distance along the
// Hilbert curve and 2D (x, y) coordinates within an NxN grid where N = 2^order.

/// Convert a 1D distance `d` along a Hilbert curve of the given `order`
/// (grid side = 2^order) to 2D coordinates (x, y).
pub fn d2xy(order: u32, d: u32) -> (u32, u32) {
    let n = 1u32 << order;
    let mut x = 0u32;
    let mut y = 0u32;
    let mut d = d;
    let mut s = 1u32;

    while s < n {
        let rx = (d / 2) & 1;
        let ry = (d ^ rx) & 1;
        rotate(s, &mut x, &mut y, rx, ry);
        x += s * rx;
        y += s * ry;
        d /= 4;
        s *= 2;
    }

    (y, x) // swapped: addresses now flow left→right first
}

/// Convert 2D coordinates (x, y) to a 1D distance `d` along a Hilbert curve
/// of the given `order`.
pub fn xy2d(order: u32, x: u32, y: u32) -> u32 {
    let (mut x, mut y) = (y, x); // match the swap in d2xy

    let n = 1u32 << order;
    let mut d = 0u32;
    let mut s = n / 2;

    while s > 0 {
        let rx = if (x & s) > 0 { 1u32 } else { 0 };
        let ry = if (y & s) > 0 { 1u32 } else { 0 };
        d += s * s * ((3 * rx) ^ ry);
        rotate(s, &mut x, &mut y, rx, ry);
        s /= 2;
    }

    d
}

/// Rotate/flip a quadrant.
fn rotate(n: u32, x: &mut u32, y: &mut u32, rx: u32, ry: u32) {
    if ry == 0 {
        if rx == 1 {
            *x = n.wrapping_sub(1).wrapping_sub(*x);
            *y = n.wrapping_sub(1).wrapping_sub(*y);
        }
        std::mem::swap(x, y);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roundtrip() {
        let order = 4;
        let n = 1u32 << order;
        for d in 0..(n * n) {
            let (x, y) = d2xy(order, d);
            let d2 = xy2d(order, x, y);
            assert_eq!(d, d2, "roundtrip failed for d={d}: ({x},{y}) -> {d2}");
        }
    }

    #[test]
    fn test_adjacency() {
        // Adjacent d values should produce adjacent (x,y) coordinates
        let order = 4;
        let n = 1u32 << order;
        for d in 0..(n * n - 1) {
            let (x1, y1) = d2xy(order, d);
            let (x2, y2) = d2xy(order, d + 1);
            let dist =
                (x1 as i32 - x2 as i32).unsigned_abs() + (y1 as i32 - y2 as i32).unsigned_abs();
            assert_eq!(
                dist,
                1,
                "d={d} and d+1={} are not adjacent: ({x1},{y1})->({x2},{y2})",
                d + 1
            );
        }
    }
}
