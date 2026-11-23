//! The menu-bar icon with a red dot while a recording runs.

/// Paints a red dot in the bottom-right corner of an RGBA image, a quarter of the icon wide.
pub fn with_dot(rgba: &[u8], w: u32, h: u32) -> Vec<u8> {
    let mut out = rgba.to_vec();
    let r = (w.min(h) / 4) as i64;
    let (cx, cy) = (w as i64 - r, h as i64 - r);
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            if (x - cx).pow(2) + (y - cy).pow(2) <= r * r {
                let i = ((y * w as i64 + x) * 4) as usize;
                out[i..i + 4].copy_from_slice(&[0xE5, 0x30, 0x30, 0xFF]);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paints_only_the_bottom_right_corner() {
        let plain = vec![10u8; 8 * 8 * 4];
        let dotted = with_dot(&plain, 8, 8);
        let px = |img: &[u8], x: usize, y: usize| img[(y * 8 + x) * 4..][..4].to_vec();
        assert_eq!(px(&dotted, 6, 6), [0xE5, 0x30, 0x30, 0xFF], "dot centre");
        assert_eq!(px(&dotted, 0, 0), [10; 4], "top-left untouched");
        assert_eq!(px(&dotted, 7, 0), [10; 4], "top-right untouched");
        assert_eq!(dotted.len(), plain.len());
        assert_eq!(plain, vec![10u8; 8 * 8 * 4], "the input is not changed");
    }
}
