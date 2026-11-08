// The battery glyph, shared by the tray icon and the executable's icon.
//
// Geometry is expressed on a 32×32 reference grid and scaled to the requested
// size, so `build.rs` can render the same artwork into the `.ico` without
// depending on the application crate. Nothing here touches Win32: the file is
// plain Rust and gets `include!`d by the build script.

/// Reference grid the coordinates below are expressed on.
pub const GRID: usize = 32;

/// Outline for dark backgrounds (dark taskbar).
pub const OUTLINE_ON_DARK: [u8; 4] = [245, 238, 235, 255];
/// Outline for light backgrounds (light taskbar).
pub const OUTLINE_ON_LIGHT: [u8; 4] = [60, 60, 60, 255];
/// Neutral outline for the executable icon: Explorer can be light or dark.
/// (Used by `build.rs`, which does not count as a use for the app binary.)
#[allow(dead_code)]
pub const OUTLINE_NEUTRAL: [u8; 4] = [120, 124, 130, 255];

/// Battery glyph pixels in BGRA, transparent outside the outline.
pub fn icon_pixels(size: usize, percent: Option<u8>, charging: bool, outline: [u8; 4]) -> Vec<u8> {
    let mut pixels = vec![0u8; size * size * 4];

    let accent = accent_color(percent, charging);
    let empty = [134u8, 126, 120, 255];
    let border = if percent.is_some() { outline } else { empty };

    // Body with a 2 px border: top, bottom, left, right, plus the terminal nub.
    fill(&mut pixels, size, 3, 9, 24, 10, border);
    fill(&mut pixels, size, 3, 22, 24, 23, border);
    fill(&mut pixels, size, 3, 9, 4, 23, border);
    fill(&mut pixels, size, 23, 9, 24, 23, border);
    fill(&mut pixels, size, 25, 13, 27, 18, border);

    if let Some(percent) = percent {
        let (left, right) = span(size, 6, 22);
        let width =
            ((right - left + 1) as f32 * (percent.min(100) as f32 / 100.0)).round() as usize;
        if width > 0 {
            let (top, bottom) = span(size, 12, 20);
            fill_pixels(
                &mut pixels,
                size,
                left,
                top,
                left + width - 1,
                bottom,
                accent,
            );
        }
    }

    pixels
}

/// Same thresholds as the card's accent colour.
pub fn accent_color(percent: Option<u8>, charging: bool) -> [u8; 4] {
    if charging {
        return [248, 189, 56, 255]; // BGRA of rgb(56, 189, 248)
    }
    match percent.unwrap_or(0) {
        p if p >= 50 => [153, 211, 52, 255], // rgb(52, 211, 153)
        p if p >= 20 => [36, 191, 251, 255], // rgb(251, 191, 36)
        _ => [113, 113, 248, 255],           // rgb(248, 113, 113)
    }
}

/// Pixel range covered by reference cells `from..=to` when scaled to `size`.
///
/// The cells are half-open on the right (`to + 1`), so neighbouring rectangles
/// stay adjacent instead of growing a gap at large sizes.
fn span(size: usize, from: usize, to: usize) -> (usize, usize) {
    let scale = size as f32 / GRID as f32;
    let start = (from as f32 * scale).round() as usize;
    let end = ((((to + 1) as f32) * scale).round() as usize).saturating_sub(1);
    (start, end.max(start))
}

fn fill(
    pixels: &mut [u8],
    size: usize,
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
    color: [u8; 4],
) {
    let (left, right) = span(size, x0, x1);
    let (top, bottom) = span(size, y0, y1);
    fill_pixels(pixels, size, left, top, right, bottom, color);
}

fn fill_pixels(
    pixels: &mut [u8],
    size: usize,
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
    color: [u8; 4],
) {
    for y in y0..=y1.min(size.saturating_sub(1)) {
        for x in x0..=x1.min(size.saturating_sub(1)) {
            let index = (y * size + x) * 4;
            pixels[index..index + 4].copy_from_slice(&color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(pixels: &[u8], size: usize, x: usize, y: usize) -> [u8; 4] {
        let index = (y * size + x) * 4;
        [
            pixels[index],
            pixels[index + 1],
            pixels[index + 2],
            pixels[index + 3],
        ]
    }

    #[test]
    fn transparent_outside_the_glyph() {
        let pixels = icon_pixels(32, Some(60), false, OUTLINE_ON_DARK);
        assert_eq!(pixel(&pixels, 32, 0, 0)[3], 0);
        assert_eq!(pixel(&pixels, 32, 31, 31)[3], 0);
    }

    #[test]
    fn fill_covers_its_share_of_the_cavity() {
        let pixels = icon_pixels(32, Some(50), false, OUTLINE_ON_DARK);
        let accent = accent_color(Some(50), false);
        assert_eq!(pixel(&pixels, 32, 7, 16), accent);
        assert_eq!(pixel(&pixels, 32, 20, 16)[3], 0);
    }

    #[test]
    fn full_battery_stops_at_the_border() {
        let pixels = icon_pixels(32, Some(100), false, OUTLINE_ON_DARK);
        let accent = accent_color(Some(100), false);
        assert_eq!(pixel(&pixels, 32, 22, 16), accent);
        assert_eq!(pixel(&pixels, 32, 23, 16), OUTLINE_ON_DARK);
    }

    #[test]
    fn outline_colour_is_taken_from_the_caller() {
        let dark = icon_pixels(32, Some(90), false, OUTLINE_ON_DARK);
        let light = icon_pixels(32, Some(90), false, OUTLINE_ON_LIGHT);
        assert_eq!(pixel(&dark, 32, 3, 16), OUTLINE_ON_DARK);
        assert_eq!(pixel(&light, 32, 3, 16), OUTLINE_ON_LIGHT);
        // The fill is theme independent.
        assert_eq!(pixel(&dark, 32, 10, 16), pixel(&light, 32, 10, 16));
    }

    #[test]
    fn charging_uses_its_own_accent() {
        assert_eq!(accent_color(Some(40), true), accent_color(Some(80), true));
        assert_ne!(accent_color(Some(40), true), accent_color(Some(40), false));
    }

    #[test]
    fn missing_controller_keeps_the_body_empty() {
        let pixels = icon_pixels(32, None, false, OUTLINE_ON_DARK);
        assert_eq!(pixel(&pixels, 32, 10, 16)[3], 0);
        assert_eq!(pixel(&pixels, 32, 3, 16), [134, 126, 120, 255]);
    }

    #[test]
    fn scales_down_to_16_pixels() {
        let pixels = icon_pixels(16, Some(100), false, OUTLINE_ON_LIGHT);
        assert_eq!(pixel(&pixels, 16, 0, 0)[3], 0);
        assert_eq!(pixel(&pixels, 16, 8, 8)[3], 255);

        // The frame stays one pixel wide and keeps the requested colour.
        let frame = pixel(&pixels, 16, 2, 8);
        assert_eq!(frame[3], 255);
        assert_ne!(frame, accent_color(Some(100), false));
    }
}
