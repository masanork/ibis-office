// SPDX-License-Identifier: AGPL-3.0-only
//! Keep vertical underlines within the gap between tightly spaced em boxes.

/// Shift the decoration toward its own text, preserving its full width. When
/// the gap is too small even for the decoration, put it against the em box;
/// neither the line pitch nor the decoration is silently resized.
pub(super) fn tight_grid_correction(size: f32, line_height: f32, distance: f32, extent: f32) -> f32 {
    let room = (line_height - size - extent).max(0.0);
    let desired = distance.min(room / 2.0);
    (distance - desired).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::tight_grid_correction;

    #[test]
    fn tight_jtd_grid_keeps_single_line_between_em_boxes() {
        let (size, pitch) = (10.45, 153_007.0 / 12_700.0);
        let distance = 0.1367 * size;
        let extent = 0.0684 * size;
        assert!(distance + extent > pitch - size); // old position overlaps preceding em box
        let corrected = distance - tight_grid_correction(size, pitch, distance, extent);
        assert!(corrected >= 0.0 && corrected + extent <= pitch - size);
        assert!((corrected - (pitch - size - extent) / 2.0).abs() < 0.0001);
    }

    #[test]
    fn ample_space_keeps_original_position_and_narrow_space_keeps_width() {
        assert_eq!(tight_grid_correction(11.14, 18.53, 1.52, 0.76), 0.0);
        assert_eq!(tight_grid_correction(10.0, 10.2, 1.0, 2.0), 1.0);
    }
}
