// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc. and masanork
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Reading-order sorting for detected text lines (horizontal and vertical).
//!
//! Implements recursive XY-cut partitioning over bounding boxes so multi-column
//! layouts and vertical Japanese text (top-to-bottom, right-to-left) are
//! ordered naturally.

/// Sort line indices in reading order given their `[x1, y1, x2, y2]` boxes.
///
/// Returns a permutation of `0..boxes.len()` ordered from first to last.
pub fn sort_reading_order(boxes: &[[u32; 4]]) -> Vec<usize> {
    if boxes.len() <= 1 {
        return (0..boxes.len()).collect();
    }
    let mut indices: Vec<usize> = (0..boxes.len()).collect();
    xy_cut_sort(boxes, &mut indices);
    indices
}

fn xy_cut_sort(boxes: &[[u32; 4]], indices: &mut [usize]) {
    if indices.len() <= 1 {
        return;
    }

    // Determine dominant orientation in this region.
    let vertical_count = indices
        .iter()
        .filter(|&&i| {
            let [x1, y1, x2, y2] = boxes[i];
            let w = x2.saturating_sub(x1);
            let h = y2.saturating_sub(y1);
            h as f32 > w as f32 * 0.8
        })
        .count();
    let is_vertical_dominant = vertical_count * 2 > indices.len();

    // Try splitting along the primary reading-block axis first:
    // - Horizontal text: try horizontal gap (Y split: top -> bottom), else vertical gap (X split: left -> right).
    // - Vertical text: try vertical gap (X split: right -> left), else horizontal gap (Y split: top -> bottom).
    if is_vertical_dominant {
        if let Some((left, right)) = try_split_x(boxes, indices) {
            // Vertical Japanese columns read right-to-left.
            let mut right_vec = right;
            let mut left_vec = left;
            xy_cut_sort(boxes, &mut right_vec);
            xy_cut_sort(boxes, &mut left_vec);
            let rlen = right_vec.len();
            indices[..rlen].copy_from_slice(&right_vec);
            indices[rlen..].copy_from_slice(&left_vec);
            return;
        }
        if let Some((top, bottom)) = try_split_y(boxes, indices) {
            let mut top_vec = top;
            let mut bottom_vec = bottom;
            xy_cut_sort(boxes, &mut top_vec);
            xy_cut_sort(boxes, &mut bottom_vec);
            let tlen = top_vec.len();
            indices[..tlen].copy_from_slice(&top_vec);
            indices[tlen..].copy_from_slice(&bottom_vec);
            return;
        }
        // Fallback: sort right-to-left by x_center, then top-to-bottom by y1.
        indices.sort_by(|&a, &b| {
            let xa = (boxes[a][0] + boxes[a][2]) / 2;
            let xb = (boxes[b][0] + boxes[b][2]) / 2;
            xb.cmp(&xa).then_with(|| boxes[a][1].cmp(&boxes[b][1]))
        });
    } else {
        if let Some((top, bottom)) = try_split_y(boxes, indices) {
            let mut top_vec = top;
            let mut bottom_vec = bottom;
            xy_cut_sort(boxes, &mut top_vec);
            xy_cut_sort(boxes, &mut bottom_vec);
            let tlen = top_vec.len();
            indices[..tlen].copy_from_slice(&top_vec);
            indices[tlen..].copy_from_slice(&bottom_vec);
            return;
        }
        if let Some((left, right)) = try_split_x(boxes, indices) {
            let mut left_vec = left;
            let mut right_vec = right;
            xy_cut_sort(boxes, &mut left_vec);
            xy_cut_sort(boxes, &mut right_vec);
            let llen = left_vec.len();
            indices[..llen].copy_from_slice(&left_vec);
            indices[llen..].copy_from_slice(&right_vec);
            return;
        }
        // Fallback: sort top-to-bottom by y_center, then left-to-right by x1.
        indices.sort_by(|&a, &b| {
            let ya = (boxes[a][1] + boxes[a][3]) / 2;
            let yb = (boxes[b][1] + boxes[b][3]) / 2;
            ya.cmp(&yb).then_with(|| boxes[a][0].cmp(&boxes[b][0]))
        });
    }
}

fn try_split_x(boxes: &[[u32; 4]], indices: &[usize]) -> Option<(Vec<usize>, Vec<usize>)> {
    let mut intervals: Vec<(u32, u32, usize)> = indices
        .iter()
        .map(|&i| (boxes[i][0], boxes[i][2], i))
        .collect();
    intervals.sort_by_key(|t| t.0);

    let mut max_end = intervals[0].1;
    let mut best_gap = 0u32;
    let mut split_x = None;

    for &(start, end, _) in &intervals[1..] {
        if start > max_end {
            let gap = start - max_end;
            if gap > best_gap {
                best_gap = gap;
                split_x = Some(max_end + gap / 2);
            }
        }
        if end > max_end {
            max_end = end;
        }
    }

    let x = split_x?;
    let mut left = Vec::new();
    let mut right = Vec::new();
    for &i in indices {
        if (boxes[i][0] + boxes[i][2]) / 2 < x {
            left.push(i);
        } else {
            right.push(i);
        }
    }
    if left.is_empty() || right.is_empty() {
        None
    } else {
        Some((left, right))
    }
}

fn try_split_y(boxes: &[[u32; 4]], indices: &[usize]) -> Option<(Vec<usize>, Vec<usize>)> {
    let mut intervals: Vec<(u32, u32, usize)> = indices
        .iter()
        .map(|&i| (boxes[i][1], boxes[i][3], i))
        .collect();
    intervals.sort_by_key(|t| t.0);

    let mut max_end = intervals[0].1;
    let mut best_gap = 0u32;
    let mut split_y = None;

    for &(start, end, _) in &intervals[1..] {
        if start > max_end {
            let gap = start - max_end;
            if gap > best_gap {
                best_gap = gap;
                split_y = Some(max_end + gap / 2);
            }
        }
        if end > max_end {
            max_end = end;
        }
    }

    let y = split_y?;
    let mut top = Vec::new();
    let mut bottom = Vec::new();
    for &i in indices {
        if (boxes[i][1] + boxes[i][3]) / 2 < y {
            top.push(i);
        } else {
            bottom.push(i);
        }
    }
    if top.is_empty() || bottom.is_empty() {
        None
    } else {
        Some((top, bottom))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn horizontal_lines_sort_top_to_bottom() {
        let boxes = [
            [20, 200, 400, 240],
            [20, 20, 400, 60],
            [20, 110, 400, 150],
        ];
        assert_eq!(sort_reading_order(&boxes), vec![1, 2, 0]);
    }

    #[test]
    fn vertical_lines_sort_right_to_left() {
        let boxes = [
            [40, 20, 80, 400],
            [300, 20, 340, 400],
            [170, 20, 210, 400],
        ];
        assert_eq!(sort_reading_order(&boxes), vec![1, 2, 0]);
    }
}
