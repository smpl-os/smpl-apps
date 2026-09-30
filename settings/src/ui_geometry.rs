pub fn dropdown_scroll_offset(
    index: i32,
    row_height: f32,
    spacing: f32,
    visible_height: f32,
    viewport_height: f32,
    current_y: f32,
) -> f32 {
    let top = index.max(0) as f32 * (row_height + spacing);
    let bottom = top + row_height;
    let offset = -current_y;
    let next = if top < offset {
        top
    } else if bottom > offset + visible_height {
        bottom - visible_height
    } else {
        offset
    };
    -next.clamp(0.0, (viewport_height - visible_height).max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_dropdown_navigation_keeps_every_selected_row_visible_down_and_up() {
        for count in [24, 100, 598] {
            for visible in [100.0, 158.0, 200.0] {
                let row = 32.0;
                let spacing = 1.0;
                let viewport = count as f32 * (row + spacing) - spacing;
                let mut y = 0.0;
                for index in (0..count).chain((0..count).rev()) {
                    y = dropdown_scroll_offset(index, row, spacing, visible, viewport, y);
                    let top = index as f32 * (row + spacing);
                    assert!(top + y >= 0.0, "row {index} above viewport");
                    assert!(top + row + y <= visible, "row {index} below viewport");
                    assert!(y <= 0.0 && y >= -(viewport - visible));
                }
                assert_eq!(y, 0.0);
                assert_eq!(
                    dropdown_scroll_offset(count - 1, row, spacing, visible, viewport, 0.0),
                    -(viewport - visible)
                );
            }
        }
    }

    #[test]
    fn short_lists_and_already_visible_rows_do_not_overscroll() {
        assert_eq!(dropdown_scroll_offset(0, 32.0, 1.0, 158.0, 32.0, 0.0), 0.0);
        assert_eq!(
            dropdown_scroll_offset(2, 32.0, 1.0, 158.0, 98.0, -80.0),
            0.0
        );
        assert_eq!(
            dropdown_scroll_offset(3, 32.0, 1.0, 158.0, 3299.0, -66.0),
            -66.0
        );
    }
}
