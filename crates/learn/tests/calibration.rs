use svanbot_learn::calibration::{Point, slope};

fn points(first: f64, second: f64) -> [Point; 2] {
    [
        Point {
            sim: 2.,
            live: first,
            radius: 1.96,
        },
        Point {
            sim: 4.,
            live: second,
            radius: 1.96,
        },
    ]
}

#[test]
fn a_live_edge_equal_to_the_sim_is_a_slope_of_one() {
    let fit = slope(&points(2., 4.)).unwrap();
    assert_eq!(fit.value, 1.);
    assert!((fit.radius - 1.96 / 20_f64.sqrt()).abs() < 1e-12);
    assert_eq!(fit.points, 2);
}

#[test]
fn a_live_edge_half_the_sim_is_a_slope_of_one_half() {
    assert_eq!(slope(&points(1., 2.)).unwrap().value, 0.5);
}

#[test]
fn fewer_than_two_points_or_an_infinite_radius_has_no_slope() {
    let mut samples = points(2., 4.);
    assert_eq!(slope(&samples[..1]), None);
    samples[1].radius = f64::INFINITY;
    assert_eq!(slope(&samples), None);
    for sample in &mut samples {
        sample.sim = 0.;
        sample.radius = 1.;
    }
    assert_eq!(slope(&samples), None);
}
