//! Deterministic arrangement rules for edited layouts.
//!
//! Live layouts are shown untouched. After a user geometry change the model
//! runs [`normalize`] once, so the canvas always shows exactly what Apply
//! will write: no overlaps, every display attached by a shared edge, and the
//! arrangement translated to start at 0,0.

/// A display's logical rectangle (Hyprland's rounded logical size).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    fn left(&self) -> i64 {
        self.x as i64
    }

    fn top(&self) -> i64 {
        self.y as i64
    }

    fn right(&self) -> i64 {
        self.x as i64 + self.w as i64
    }

    fn bottom(&self) -> i64 {
        self.y as i64 + self.h as i64
    }

    fn overlap_x(&self, other: &Rect) -> i64 {
        self.right().min(other.right()) - self.left().max(other.left())
    }

    fn overlap_y(&self, other: &Rect) -> i64 {
        self.bottom().min(other.bottom()) - self.top().max(other.top())
    }

    pub fn overlaps(&self, other: &Rect) -> bool {
        self.overlap_x(other) > 0 && self.overlap_y(other) > 0
    }

    /// The rectangles touch along an edge of positive length.
    pub fn shares_edge(&self, other: &Rect) -> bool {
        let side_by_side = self.right() == other.left() || other.right() == self.left();
        let stacked = self.bottom() == other.top() || other.bottom() == self.top();
        (side_by_side && self.overlap_y(other) > 0) || (stacked && self.overlap_x(other) > 0)
    }

    pub fn contains(&self, x: i64, y: i64) -> bool {
        (self.left()..self.right()).contains(&x) && (self.top()..self.bottom()).contains(&y)
    }
}

/// Nearest value to `target` in `[low, high]` outside every forbidden
/// (inclusive) interval; ties prefer the lower value.
fn nearest_allowed(
    low: i64,
    high: i64,
    mut forbidden: Vec<(i64, i64)>,
    target: i64,
) -> Option<i64> {
    forbidden.sort_unstable();
    let mut best: Option<i64> = None;
    let mut consider = |start: i64, end: i64| {
        if start > end {
            return;
        }
        let candidate = target.clamp(start, end);
        if best.is_none_or(|b| (candidate - target).abs() < (b - target).abs()) {
            best = Some(candidate);
        }
    };
    let mut cursor = low;
    for (start, end) in forbidden {
        if end < cursor {
            continue;
        }
        if start > high {
            break;
        }
        consider(cursor, start - 1);
        cursor = cursor.max(end + 1);
    }
    consider(cursor, high);
    best
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Right,
    Left,
    Below,
    Above,
}

const SIDES: [Side; 4] = [Side::Right, Side::Left, Side::Below, Side::Above];

/// The anchor side facing `rect`: the axis on which their centers are
/// further apart relative to the combined sizes (AABB separation axis).
fn facing_side(anchor: &Rect, rect: &Rect) -> Side {
    let dx = (2 * rect.left() + rect.w as i64) - (2 * anchor.left() + anchor.w as i64);
    let dy = (2 * rect.top() + rect.h as i64) - (2 * anchor.top() + anchor.h as i64);
    if dx.abs() * (anchor.h as i64 + rect.h as i64) >= dy.abs() * (anchor.w as i64 + rect.w as i64)
    {
        if dx >= 0 {
            Side::Right
        } else {
            Side::Left
        }
    } else if dy >= 0 {
        Side::Below
    } else {
        Side::Above
    }
}

/// The closest position for `rect` that overlaps nothing placed and shares
/// an edge with a placed rectangle. Positions on the side of an anchor that
/// faces `rect` win, so a neighbor pushed by a growing display stays beside
/// it; any other side is only used when every facing side is blocked.
fn nearest_attachment(rect: Rect, placed: &[Rect]) -> Rect {
    nearest_on_sides(rect, placed, true)
        .or_else(|| nearest_on_sides(rect, placed, false))
        .unwrap_or(rect)
}

fn nearest_on_sides(rect: Rect, placed: &[Rect], facing_only: bool) -> Option<Rect> {
    let mut best: Option<(i64, Rect)> = None;
    for anchor in placed {
        let facing = facing_side(anchor, &rect);
        for side in SIDES {
            if facing_only && side != facing {
                continue;
            }
            // (fixed coordinate, sliding range, horizontal edge?)
            let (fixed, low, high, horizontal) = match side {
                Side::Right => (
                    anchor.right(),
                    anchor.top() - rect.h as i64 + 1,
                    anchor.bottom() - 1,
                    false,
                ),
                Side::Left => (
                    anchor.left() - rect.w as i64,
                    anchor.top() - rect.h as i64 + 1,
                    anchor.bottom() - 1,
                    false,
                ),
                Side::Below => (
                    anchor.bottom(),
                    anchor.left() - rect.w as i64 + 1,
                    anchor.right() - 1,
                    true,
                ),
                Side::Above => (
                    anchor.top() - rect.h as i64,
                    anchor.left() - rect.w as i64 + 1,
                    anchor.right() - 1,
                    true,
                ),
            };
            let forbidden = placed
                .iter()
                .filter_map(|other| {
                    if horizontal {
                        let probe = Rect {
                            y: fixed as i32,
                            ..rect
                        };
                        (probe.overlap_y(other) > 0)
                            .then(|| (other.left() - rect.w as i64 + 1, other.right() - 1))
                    } else {
                        let probe = Rect {
                            x: fixed as i32,
                            ..rect
                        };
                        (probe.overlap_x(other) > 0)
                            .then(|| (other.top() - rect.h as i64 + 1, other.bottom() - 1))
                    }
                })
                .collect();
            let current = if horizontal { rect.left() } else { rect.top() };
            // A diagonal (corner) position would clamp to a 1-pixel contact;
            // aim for the flush end of the edge instead.
            let (start, length) = if horizontal {
                (anchor.left(), rect.w as i64)
            } else {
                (anchor.top(), rect.h as i64)
            };
            let end = if horizontal {
                anchor.right()
            } else {
                anchor.bottom()
            };
            let aim = if current < low {
                start.min(end - length)
            } else if current > high {
                start.max(end - length)
            } else {
                current
            };
            let Some(slide) = nearest_allowed(low, high, forbidden, aim) else {
                continue;
            };
            let candidate = if horizontal {
                Rect {
                    x: slide as i32,
                    y: fixed as i32,
                    ..rect
                }
            } else {
                Rect {
                    x: fixed as i32,
                    y: slide as i32,
                    ..rect
                }
            };
            let dx = candidate.left() - rect.left();
            let dy = candidate.top() - rect.top();
            let distance = dx * dx + dy * dy;
            if best.is_none_or(|(d, _)| distance < d) {
                best = Some((distance, candidate));
            }
        }
    }
    best.map(|(_, candidate)| candidate)
}

/// The first display in (x, y) order stays. The arrangement then grows by
/// connectivity: the next display (in (x, y) order) that shares an edge with
/// the placed ones without overlapping them is kept where it is. Only when no
/// remaining display qualifies does the first remaining one move, to the
/// nearest position that does (preferring the side that faces it; a corner
/// position becomes flush rather than a 1-pixel contact). A valid layout is
/// therefore never rearranged. Finally the arrangement is translated so
/// min x = min y = 0.
pub fn normalize(rects: &[Rect]) -> Vec<Rect> {
    let mut result = rects.to_vec();
    let mut pending: Vec<usize> = (0..result.len()).collect();
    pending.sort_by_key(|&i| (rects[i].x, rects[i].y));
    let mut placed: Vec<Rect> = Vec::with_capacity(result.len());
    while !pending.is_empty() {
        let attached = pending.iter().position(|&index| {
            let rect = result[index];
            placed.iter().any(|p| rect.shares_edge(p)) && !placed.iter().any(|p| rect.overlaps(p))
        });
        let position = match attached {
            Some(position) => position,
            None => {
                if !placed.is_empty() {
                    result[pending[0]] = nearest_attachment(result[pending[0]], &placed);
                }
                0
            }
        };
        placed.push(result[pending.remove(position)]);
    }
    let min_x = result.iter().map(|r| r.x).min().unwrap_or(0);
    let min_y = result.iter().map(|r| r.y).min().unwrap_or(0);
    for rect in &mut result {
        rect.x -= min_x;
        rect.y -= min_y;
    }
    result
}

/// Magnetic drop: align a dragged display with nearby edges of the others
/// when within `threshold` logical pixels.
pub fn snap_drop(others: &[Rect], rect: Rect, threshold: i32) -> Rect {
    let nearest = |current: i32, candidates: &mut dyn Iterator<Item = i64>| {
        candidates
            .map(|c| (((c - current as i64).abs()), c))
            .filter(|(distance, _)| *distance <= threshold as i64)
            .min()
            .map_or(current, |(_, c)| c as i32)
    };
    let x = nearest(
        rect.x,
        &mut others.iter().flat_map(|o| {
            [
                o.left(),
                o.right(),
                o.left() - rect.w as i64,
                o.right() - rect.w as i64,
            ]
        }),
    );
    let y = nearest(
        rect.y,
        &mut others.iter().flat_map(|o| {
            [
                o.top(),
                o.bottom(),
                o.top() - rect.h as i64,
                o.bottom() - rect.h as i64,
            ]
        }),
    );
    Rect { x, y, ..rect }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    fn assert_valid(rects: &[Rect]) {
        for (i, a) in rects.iter().enumerate() {
            for b in &rects[i + 1..] {
                assert!(!a.overlaps(b), "{a:?} overlaps {b:?}");
            }
            if rects.len() > 1 {
                assert!(
                    rects.iter().any(|b| b != a && a.shares_edge(b)),
                    "{a:?} is detached"
                );
            }
        }
        assert_eq!(rects.iter().map(|r| r.x).min(), Some(0));
        assert_eq!(rects.iter().map(|r| r.y).min(), Some(0));
    }

    #[test]
    fn valid_layouts_are_unchanged_including_vertical_stacks() {
        let side = [rect(0, 0, 2560, 1440), rect(2560, 0, 1080, 1920)];
        assert_eq!(normalize(&side), side);
        let stack = [rect(0, 0, 1920, 1080), rect(0, 1080, 1920, 1080)];
        assert_eq!(normalize(&stack), stack);
        let offset = [rect(0, 200, 1920, 1080), rect(1920, 0, 1920, 1080)];
        assert_eq!(normalize(&offset), offset);
    }

    #[test]
    fn rotation_keeps_adjacency_and_closes_gaps() {
        // DP-3 rotated to portrait still touches HDMI-A-1.
        let rotated_right = [rect(0, 0, 2560, 1440), rect(2560, 0, 1080, 1920)];
        assert_eq!(normalize(&rotated_right), rotated_right);
        // HDMI-A-1 rotated to portrait leaves a gap; DP-3 slides left.
        let rotated_left = normalize(&[rect(0, 0, 1440, 2560), rect(2560, 0, 1920, 1080)]);
        assert_eq!(
            rotated_left,
            [rect(0, 0, 1440, 2560), rect(1440, 0, 1920, 1080)]
        );
        assert_valid(&rotated_left);
    }

    #[test]
    fn shrinking_the_left_display_closes_the_gap() {
        let shrunk = normalize(&[rect(0, 0, 1920, 1080), rect(2560, 0, 1920, 1080)]);
        assert_eq!(shrunk, [rect(0, 0, 1920, 1080), rect(1920, 0, 1920, 1080)]);
    }

    #[test]
    fn growing_display_pushes_its_neighbor_instead_of_overlapping() {
        let grown = normalize(&[rect(0, 0, 3840, 2160), rect(2560, 0, 1920, 1080)]);
        assert_eq!(grown, [rect(0, 0, 3840, 2160), rect(3840, 0, 1920, 1080)]);
    }

    #[test]
    fn overlap_after_drag_is_resolved_and_translated_to_origin() {
        let dragged = normalize(&[rect(0, 0, 2560, 1440), rect(-1900, 100, 1920, 1080)]);
        assert_valid(&dragged);
        assert_eq!(
            dragged,
            [rect(1920, 0, 2560, 1440), rect(0, 100, 1920, 1080)]
        );
        let detached = normalize(&[rect(500, 300, 1920, 1080), rect(5000, 4000, 1920, 1080)]);
        assert_valid(&detached);
        assert_eq!(
            detached,
            [rect(0, 0, 1920, 1080), rect(0, 1080, 1920, 1080)]
        );
    }

    #[test]
    fn corner_positions_become_flush_instead_of_one_pixel_contacts() {
        let corner = normalize(&[rect(0, 0, 2560, 1440), rect(-1080, 1440, 1080, 1920)]);
        assert_valid(&corner);
        assert_eq!(corner, [rect(1080, 0, 2560, 1440), rect(0, 0, 1080, 1920)]);
    }

    #[test]
    fn three_display_layouts_stay_valid_and_deterministic() {
        let input = [
            rect(0, 0, 1920, 1080),
            rect(100, 50, 2560, 1440),
            rect(3000, 3000, 1080, 1920),
        ];
        let once = normalize(&input);
        assert_valid(&once);
        assert_eq!(normalize(&input), once);
        assert_eq!(normalize(&once), once);
    }

    #[test]
    fn displays_attached_through_a_later_display_are_not_moved() {
        // eDP-1 touches only DP-2, which comes after it in (x, y) order.
        let layout = [
            rect(0, 0, 1920, 1080),
            rect(1920, 0, 2560, 1440),
            rect(1800, 1440, 1920, 1200),
        ];
        assert_eq!(normalize(&layout), layout);
    }

    struct Random(u64);

    impl Random {
        fn next(&mut self, bound: i64) -> i64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % bound as u64) as i64
        }
    }

    const SIZES: [(i32, i32); 6] = [
        (1920, 1080),
        (1080, 1920),
        (2560, 1440),
        (1440, 900),
        (1600, 900),
        (1280, 1024),
    ];

    /// A random connected, non-overlapping layout translated to the origin.
    fn connected_layout(random: &mut Random, count: usize) -> Vec<Rect> {
        let (w, h) = SIZES[random.next(6) as usize];
        let mut rects = vec![rect(0, 0, w, h)];
        while rects.len() < count {
            let (w, h) = SIZES[random.next(6) as usize];
            let anchor = rects[random.next(rects.len() as i64) as usize];
            let candidate = match random.next(4) {
                0 => rect(
                    anchor.x + anchor.w,
                    anchor.y - h + 1 + random.next((anchor.h + h - 1) as i64) as i32,
                    w,
                    h,
                ),
                1 => rect(
                    anchor.x - w,
                    anchor.y - h + 1 + random.next((anchor.h + h - 1) as i64) as i32,
                    w,
                    h,
                ),
                2 => rect(
                    anchor.x - w + 1 + random.next((anchor.w + w - 1) as i64) as i32,
                    anchor.y + anchor.h,
                    w,
                    h,
                ),
                _ => rect(
                    anchor.x - w + 1 + random.next((anchor.w + w - 1) as i64) as i32,
                    anchor.y - h,
                    w,
                    h,
                ),
            };
            if !rects.iter().any(|r| r.overlaps(&candidate)) {
                rects.push(candidate);
            }
        }
        let min_x = rects.iter().map(|r| r.x).min().unwrap();
        let min_y = rects.iter().map(|r| r.y).min().unwrap();
        rects
            .iter()
            .map(|r| rect(r.x - min_x, r.y - min_y, r.w, r.h))
            .collect()
    }

    #[test]
    fn random_valid_layouts_are_never_rearranged() {
        let mut random = Random(0x5eed_1234_abcd_0001);
        for _ in 0..2000 {
            let count = 2 + random.next(3) as usize;
            let layout = connected_layout(&mut random, count);
            assert_eq!(normalize(&layout), layout);
        }
    }

    #[test]
    fn random_arrangements_normalize_to_valid_fixed_points() {
        let mut random = Random(0x0dd_ba11_cafe_f00d);
        for _ in 0..2000 {
            let count = 2 + random.next(3) as usize;
            let input: Vec<Rect> = (0..count)
                .map(|_| {
                    let (w, h) = SIZES[random.next(6) as usize];
                    rect(
                        random.next(10_000) as i32 - 5000,
                        random.next(10_000) as i32 - 5000,
                        w,
                        h,
                    )
                })
                .collect();
            let once = normalize(&input);
            assert_valid(&once);
            assert_eq!(normalize(&once), once, "not idempotent for {input:?}");
        }
    }

    #[test]
    fn magnetic_drop_aligns_with_nearby_edges_only() {
        let others = [rect(0, 0, 2560, 1440)];
        assert_eq!(
            snap_drop(&others, rect(2575, 12, 1920, 1080), 40),
            rect(2560, 0, 1920, 1080)
        );
        assert_eq!(
            snap_drop(&others, rect(2700, 300, 1920, 1080), 40),
            rect(2700, 300, 1920, 1080)
        );
    }

    #[test]
    fn nearest_allowed_skips_forbidden_ranges() {
        assert_eq!(nearest_allowed(0, 10, vec![(3, 6)], 4), Some(2));
        assert_eq!(nearest_allowed(0, 10, vec![(3, 6)], 6), Some(7));
        assert_eq!(nearest_allowed(0, 10, vec![(0, 10)], 4), None);
        assert_eq!(nearest_allowed(0, 10, vec![], 40), Some(10));
    }
}
