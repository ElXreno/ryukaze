//! Which key on the picture each marker is: the scale and shift from CSS
//! pixels to the picture's searched for, the most markers on keys of their
//! own, then fitted by least squares on the keys' centres.

use std::collections::BTreeMap;

use crate::boxes::Patch;

/// From CSS pixels to the picture's, on each axis: `at + scale * css`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub at: (f32, f32),
    pub scale: (f32, f32),
}

impl Transform {
    fn apply(&self, (x, y): (f32, f32)) -> (f32, f32) {
        (self.at.0 + self.scale.0 * x, self.at.1 + self.scale.1 * y)
    }
}

/// The markers placed on keys.
#[derive(Debug, Clone, PartialEq)]
pub struct Fit {
    pub transform: Transform,
    pub placed: BTreeMap<u16, Patch>,
    /// The markers on no key, or on one another's.
    pub unplaced: Vec<u16>,
    /// The keys no marker is on.
    pub spare: usize,
}

/// The keys under each pixel of a picture.
struct Grid {
    width: usize,
    height: usize,
    owner: Vec<u32>,
}

impl Grid {
    fn new(width: usize, height: usize, keys: &[Patch]) -> Self {
        let mut owner = vec![0; width * height];
        for (index, key) in keys.iter().enumerate() {
            for y in key.y0 as usize..(key.y1 as usize).min(height) {
                for x in key.x0 as usize..(key.x1 as usize).min(width) {
                    owner[y * width + x] = index as u32 + 1;
                }
            }
        }
        Self {
            width,
            height,
            owner,
        }
    }

    fn at(&self, (x, y): (f32, f32)) -> Option<usize> {
        if !(x >= 0.0 && y >= 0.0) {
            return None;
        }
        let (x, y) = (x as usize, y as usize);
        if x >= self.width || y >= self.height {
            return None;
        }
        self.owner[y * self.width + x]
            .checked_sub(1)
            .map(|i| i as usize)
    }
}

/// How the scale is searched around a first guess, as a fraction of it.
const SCALE_STEPS: [f32; 9] = [0.94, 0.955, 0.97, 0.985, 1.0, 1.015, 1.03, 1.045, 1.06];

/// How many of the best guesses are fitted.
const KEPT: usize = 8;

/// The markers, by HID usage with their centres, on the `keys` of a
/// picture `width` by `height`, its box `size` in CSS pixels if known.
pub fn fit(
    markers: &BTreeMap<u16, (f32, f32)>,
    keys: &[Patch],
    width: usize,
    height: usize,
    size: Option<(f32, f32)>,
) -> Option<Fit> {
    if markers.len() < 2 || keys.len() < 2 {
        return None;
    }
    let grid = Grid::new(width, height, keys);
    let centres: Vec<(f32, f32)> = markers.values().copied().collect();
    let span = |values: &mut dyn Iterator<Item = f32>| {
        values.fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(v), hi.max(v)))
    };
    let (mx, my) = (
        span(&mut centres.iter().map(|c| c.0)),
        span(&mut centres.iter().map(|c| c.1)),
    );
    let (kx, ky) = (
        span(&mut keys.iter().map(|k| k.centre().0)),
        span(&mut keys.iter().map(|k| k.centre().1)),
    );
    let ratio = |keys: (f32, f32), markers: (f32, f32)| {
        let across = markers.1 - markers.0;
        (across > 0.0).then(|| (keys.1 - keys.0) / across)
    };
    let mut guesses = Vec::new();
    if let Some((w, h)) = size.filter(|(w, h)| *w > 0.0 && *h > 0.0) {
        guesses.push((width as f32 / w, height as f32 / h));
    }
    if let (Some(sx), Some(sy)) = (ratio(kx, mx), ratio(ky, my)) {
        guesses.push((sx, sy));
    }
    let mut sides: Vec<u32> = keys.iter().map(|k| k.width().min(k.height())).collect();
    sides.sort_unstable();
    let step = (sides[sides.len() / 2] as f32 / 4.0).max(1.0);

    let mut tried: Vec<(usize, Transform)> = Vec::new();
    for (gx, gy) in guesses {
        for fx in SCALE_STEPS {
            for fy in SCALE_STEPS {
                let scale = (gx * fx, gy * fy);
                let from = (kx.0 - scale.0 * mx.1, ky.0 - scale.1 * my.1);
                let to = (kx.1 - scale.0 * mx.0, ky.1 - scale.1 * my.0);
                let mut ax = from.0;
                while ax <= to.0 {
                    let mut ay = from.1;
                    while ay <= to.1 {
                        let transform = Transform {
                            at: (ax, ay),
                            scale,
                        };
                        let hits = distinct_hits(&grid, &centres, &transform);
                        if tried.len() < KEPT || hits > tried[KEPT - 1].0 {
                            let at = tried.partition_point(|(other, _)| *other >= hits);
                            tried.insert(at, (hits, transform));
                            tried.truncate(KEPT);
                        }
                        ay += step;
                    }
                    ax += step;
                }
            }
        }
    }
    tried
        .into_iter()
        .map(|(_, transform)| refine(markers, keys, &grid, transform))
        .max_by(|a, b| {
            (a.placed.len(), std::cmp::Reverse(a.spare))
                .cmp(&(b.placed.len(), std::cmp::Reverse(b.spare)))
        })
}

/// How many keys the markers land on, each counted once.
fn distinct_hits(grid: &Grid, centres: &[(f32, f32)], transform: &Transform) -> usize {
    let mut hit: Vec<usize> = centres
        .iter()
        .filter_map(|&centre| grid.at(transform.apply(centre)))
        .collect();
    hit.sort_unstable();
    hit.dedup();
    hit.len()
}

/// The markers on keys of their own under `transform`, then the transform
/// fitted to them, until they stay.
fn refine(
    markers: &BTreeMap<u16, (f32, f32)>,
    keys: &[Patch],
    grid: &Grid,
    mut transform: Transform,
) -> Fit {
    let mut last = None;
    for _ in 0..20 {
        let placed = place(markers, keys, grid, &transform);
        if last.as_ref() == Some(&placed) {
            break;
        }
        let pairs: Vec<((f32, f32), (f32, f32))> = placed
            .iter()
            .map(|(code, &key)| (markers[code], keys[key].centre()))
            .collect();
        if let (Some(x), Some(y)) = (
            line(pairs.iter().map(|(m, k)| (m.0, k.0))),
            line(pairs.iter().map(|(m, k)| (m.1, k.1))),
        ) {
            transform = Transform {
                at: (x.0, y.0),
                scale: (x.1, y.1),
            };
        }
        last = Some(placed);
    }
    let placed = place(markers, keys, grid, &transform);
    let unplaced = markers
        .keys()
        .filter(|code| !placed.contains_key(code))
        .copied()
        .collect();
    let spare = keys.len() - placed.len();
    Fit {
        transform,
        placed: placed
            .into_iter()
            .map(|(code, key)| (code, keys[key]))
            .collect(),
        unplaced,
        spare,
    }
}

/// Each marker on the key under it, where no other marker is nearer its
/// centre.
fn place(
    markers: &BTreeMap<u16, (f32, f32)>,
    keys: &[Patch],
    grid: &Grid,
    transform: &Transform,
) -> BTreeMap<u16, usize> {
    let mut best: BTreeMap<usize, (f32, u16)> = BTreeMap::new();
    for (&code, &centre) in markers {
        let at = transform.apply(centre);
        let Some(key) = grid.at(at) else {
            continue;
        };
        let (cx, cy) = keys[key].centre();
        let off = (at.0 - cx).hypot(at.1 - cy);
        if best.get(&key).is_none_or(|(other, _)| off < *other) {
            best.insert(key, (off, code));
        }
    }
    best.into_iter()
        .map(|(key, (_, code))| (code, key))
        .collect()
}

/// The least-squares line through `points`: where it crosses 0, and its
/// slope.
fn line(points: impl Iterator<Item = (f32, f32)>) -> Option<(f32, f32)> {
    let (mut n, mut sx, mut sy, mut sxx, mut sxy) = (0.0f64, 0.0, 0.0, 0.0, 0.0);
    for (x, y) in points {
        let (x, y) = (f64::from(x), f64::from(y));
        n += 1.0;
        sx += x;
        sy += y;
        sxx += x * x;
        sxy += x * y;
    }
    let spread = n * sxx - sx * sx;
    if n < 2.0 || spread.abs() < 1e-9 {
        return None;
    }
    let slope = (n * sxy - sx * sy) / spread;
    Some((((sy - slope * sx) / n) as f32, slope as f32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn places_markers_scaled_and_shifted() {
        // A grid of keys 30 apart, markers on them 10 CSS pixels apart.
        let mut keys = Vec::new();
        let mut markers = BTreeMap::new();
        for row in 0..4u32 {
            for column in 0..10u32 {
                let (x, y) = (20 + 30 * column, 15 + 30 * row);
                keys.push(Patch {
                    x0: x,
                    y0: y,
                    x1: x + 26,
                    y1: y + 26,
                });
                let code = (row * 10 + column) as u16;
                // 3 px a CSS pixel, 20 px over and 15 down, and off by a bit.
                let wobble = if column % 2 == 0 { 0.4 } else { -0.3 };
                markers.insert(
                    code,
                    (column as f32 * 10.0 + 4.3 + wobble, row as f32 * 10.0 + 4.3),
                );
            }
        }
        // A key with no marker.
        keys.push(Patch {
            x0: 350,
            y0: 15,
            x1: 376,
            y1: 41,
        });
        let fit = fit(&markers, &keys, 400, 150, Some((130.0, 50.0))).unwrap();
        assert_eq!(fit.placed.len(), 40);
        assert!(fit.unplaced.is_empty());
        assert_eq!(fit.spare, 1);
        assert_eq!(fit.placed[&13], keys[13]);
        assert!((fit.transform.scale.0 - 3.0).abs() < 0.05, "{fit:?}");
    }

    #[test]
    fn two_markers_on_one_key_keep_the_nearer() {
        let keys: Vec<Patch> = (0..5)
            .map(|i| Patch {
                x0: 10 + 40 * i,
                y0: 10,
                x1: 46 + 40 * i,
                y1: 46,
            })
            .collect();
        let mut markers: BTreeMap<u16, (f32, f32)> = (0..5u16)
            .map(|i| (i, (28.0 + 40.0 * f32::from(i), 28.0)))
            .collect();
        markers.insert(9, (40.0, 20.0));
        let fit = fit(&markers, &keys, 220, 60, Some((220.0, 60.0))).unwrap();
        assert_eq!(fit.placed.len(), 5);
        assert_eq!(fit.unplaced, [9]);
    }

    #[test]
    fn fits_a_line() {
        let (at, slope) = line([(0.0, 1.0), (1.0, 3.0), (2.0, 5.0)].into_iter()).unwrap();
        assert!((at - 1.0).abs() < 1e-5 && (slope - 2.0).abs() < 1e-5);
        assert_eq!(line([(1.0, 1.0)].into_iter()), None);
    }
}
