//! Port of `git_of_theseus.utils.generate_n_colors`.
//!
//! Reproduces the greedy farthest-point-in-RGB palette used by the Python
//! plot scripts. Two subtleties matter for byte-for-byte parity with the
//! Python output:
//!
//! 1. `numpy.linspace(0.4, 0.9, 6)` is *not* exactly `[0.4, 0.5, ..., 0.9]`
//!    in floating point — element 2 is `0.6000000000000001` and element 3
//!    is `0.7000000000000001`. We use the same start + step computation
//!    so the resulting RGB tuples compare equal to the Python ones.
//! 2. `python max(iterable, key=...)` returns the *first* element whose
//!    key is maximal, while Rust's `Iterator::max_by` returns the *last*.
//!    We implement first-wins explicitly.
//!
//! See `tests/colors.rs` for a snapshot test against the Python output.
//!
//! ```text
//! >>> generate_n_colors(8)
//! [(0.9,0.4,0.4), (0.4,0.9,0.9), (0.4,0.4,0.6), (0.6,0.9,0.4), ...]
//! ```

use plotters::style::RGBColor;

/// Returns `n` RGB triples in `[0, 1]^3`, matching the Python implementation.
pub fn generate_n_colors_f64(n: usize) -> Vec<(f64, f64, f64)> {
    // numpy.linspace(0.4, 0.9, 6): start + i * (stop - start) / (num - 1).
    let start = 0.4_f64;
    let stop = 0.9_f64;
    let num = 6_usize;
    let step = (stop - start) / (num as f64 - 1.0);
    let vs: Vec<f64> = (0..num).map(|i| start + i as f64 * step).collect();

    let mut colors: Vec<(f64, f64, f64)> = vec![(0.9, 0.4, 0.4)];
    if n == 0 {
        return Vec::new();
    }

    while colors.len() < n {
        // Iterate the 6^3 grid in lexicographic (v0, v1, v2) order, which is
        // exactly what `itertools.product(vs, vs, vs)` yields in Python.
        let mut best: Option<((f64, f64, f64), f64)> = None;
        for &r in &vs {
            for &g in &vs {
                for &b in &vs {
                    let cand = (r, g, b);
                    let key = colors
                        .iter()
                        .map(|&c| sq_dist(cand, c))
                        .fold(f64::INFINITY, f64::min);
                    match best {
                        // Strictly greater wins -> first occurrence of the
                        // max is retained (matches Python `max`).
                        Some((_, best_key)) if key <= best_key => {}
                        _ => best = Some((cand, key)),
                    }
                }
            }
        }
        colors.push(best.expect("grid is non-empty").0);
    }

    colors.truncate(n);
    colors
}

/// Convenience wrapper returning `plotters` colors.
pub fn generate_n_colors(n: usize) -> Vec<RGBColor> {
    generate_n_colors_f64(n)
        .into_iter()
        .map(|(r, g, b)| {
            RGBColor(
                (r * 255.0).round() as u8,
                (g * 255.0).round() as u8,
                (b * 255.0).round() as u8,
            )
        })
        .collect()
}

fn sq_dist(a: (f64, f64, f64), b: (f64, f64, f64)) -> f64 {
    let dr = a.0 - b.0;
    let dg = a.1 - b.1;
    let db = a.2 - b.2;
    dr * dr + dg * dg + db * db
}
