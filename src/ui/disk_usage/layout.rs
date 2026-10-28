// SPDX-License-Identifier: MIT

#[derive(Clone, Copy, Debug)]
pub(super) struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub fn contains(self, x: f64, y: f64) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.width && y < self.y + self.height
    }
}

pub(super) fn squarify(weights: &[u64], width: f64, height: f64) -> Vec<Rect> {
    if width <= 0.0 || height <= 0.0 || !width.is_finite() || !height.is_finite() {
        return Vec::new();
    }
    let sum: f64 = weights.iter().map(|weight| *weight as f64).sum();
    if sum <= 0.0 {
        return Vec::new();
    }
    let areas: Vec<_> = weights
        .iter()
        .map(|weight| *weight as f64 / sum * width * height)
        .collect();
    let mut remaining = Rect {
        x: 0.0,
        y: 0.0,
        width,
        height,
    };
    let mut output = Vec::with_capacity(weights.len());
    let mut start = 0;
    while start < areas.len() {
        let side = remaining.width.min(remaining.height);
        let mut end = start + 1;
        while end < areas.len()
            && worst(&areas[start..=end], side) <= worst(&areas[start..end], side)
        {
            end += 1;
        }
        let total: f64 = areas[start..end].iter().sum();
        if remaining.width >= remaining.height {
            let width = (total / remaining.height.max(f64::MIN_POSITIVE)).min(remaining.width);
            let mut y = remaining.y;
            for area in &areas[start..end] {
                let height = (*area / width.max(f64::MIN_POSITIVE))
                    .min(remaining.y + remaining.height - y)
                    .max(0.0);
                output.push(Rect {
                    x: remaining.x,
                    y,
                    width,
                    height,
                });
                y += height;
            }
            remaining.x += width;
            remaining.width = (remaining.width - width).max(0.0);
        } else {
            let height = (total / remaining.width.max(f64::MIN_POSITIVE)).min(remaining.height);
            let mut x = remaining.x;
            for area in &areas[start..end] {
                let width = (*area / height.max(f64::MIN_POSITIVE))
                    .min(remaining.x + remaining.width - x)
                    .max(0.0);
                output.push(Rect {
                    x,
                    y: remaining.y,
                    width,
                    height,
                });
                x += width;
            }
            remaining.y += height;
            remaining.height = (remaining.height - height).max(0.0);
        }
        start = end;
    }
    output
}

fn worst(areas: &[f64], side: f64) -> f64 {
    let sum: f64 = areas.iter().sum();
    let minimum = areas.iter().copied().fold(f64::INFINITY, f64::min);
    let maximum = areas.iter().copied().fold(0.0, f64::max);
    if minimum <= 0.0 || side <= 0.0 {
        return f64::INFINITY;
    }
    let square = sum * sum;
    (side * side * maximum / square).max(square / (side * side * minimum))
}
