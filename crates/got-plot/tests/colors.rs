//! Snapshot of `generate_n_colors_f64` against the Python reference output.
//!
//! Reference values are produced by:
//!     python -c "from git_of_theseus.utils import generate_n_colors;
//!                import json; print(json.dumps(generate_n_colors(8)))"

use got_plot::colors::generate_n_colors_f64;

fn approx_eq(a: (f64, f64, f64), b: (f64, f64, f64)) -> bool {
    (a.0 - b.0).abs() < 1e-12 && (a.1 - b.1).abs() < 1e-12 && (a.2 - b.2).abs() < 1e-12
}

#[test]
fn matches_python_first_eight() {
    let expected: [(f64, f64, f64); 8] = [
        (0.9, 0.4, 0.4),
        (0.4, 0.9, 0.9),
        (0.4, 0.4, 0.6000000000000001),
        (0.6000000000000001, 0.9, 0.4),
        (0.9, 0.6000000000000001, 0.9),
        (0.8, 0.9, 0.7000000000000001),
        (0.6000000000000001, 0.4, 0.9),
        (0.7000000000000001, 0.6000000000000001, 0.6000000000000001),
    ];
    let got = generate_n_colors_f64(8);
    assert_eq!(got.len(), 8);
    for (i, (g, e)) in got.iter().zip(expected.iter()).enumerate() {
        assert!(approx_eq(*g, *e), "color {i}: got {g:?}, expected {e:?}");
    }
}

#[test]
fn first_color_is_seed() {
    assert_eq!(generate_n_colors_f64(1), vec![(0.9, 0.4, 0.4)]);
}

#[test]
fn handles_zero() {
    assert!(generate_n_colors_f64(0).is_empty());
}
