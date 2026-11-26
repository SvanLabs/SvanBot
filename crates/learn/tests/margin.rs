use svanbot_learn::margin::{Admission, MARGIN, pooled, read};

#[test]
fn pooled_averages_each_bouts_difference_over_the_rows() {
    let base: &[&[f64]] = &[&[10., 20.], &[0., 0.]];
    let next: &[&[f64]] = &[&[15., 30.], &[4., 4.]];
    assert_eq!(pooled(base, next), Some(vec![9_000.0, 14_000.0]));
}

#[test]
fn pooled_refuses_runs_that_are_not_the_same_bouts() {
    assert_eq!(pooled(&[], &[]), None);
    assert_eq!(pooled(&[&[]], &[&[]]), None);
    assert_eq!(pooled(&[&[1.]], &[&[1.], &[2.]]), None);
    assert_eq!(pooled(&[&[1., 2.]], &[&[1.]]), None);
    assert_eq!(pooled(&[&[1.], &[1., 2.]], &[&[1.], &[1., 2.]]), None);
}

#[test]
fn a_policy_inside_the_margin_is_admitted() {
    assert_eq!(read(&[-5_000.; 60], MARGIN).admission, Admission::Admitted);
}

#[test]
fn a_policy_wholly_below_minus_the_margin_is_refused() {
    assert_eq!(read(&[-40_000.; 60], MARGIN).admission, Admission::Refused);
}

#[test]
fn too_few_bouts_are_not_yet_read() {
    let reading = read(&[-40_000.; 10], MARGIN);
    assert_eq!(reading.admission, Admission::Unresolved);
    assert_eq!(reading.bouts, 10);
}

#[test]
fn a_wide_interval_is_unresolved_not_admitted() {
    let differences = [150_000., -150_000.].repeat(20);
    assert_eq!(read(&differences, MARGIN).admission, Admission::Unresolved);
}

#[test]
fn a_wide_interval_wholly_below_minus_the_margin_is_still_refused() {
    let differences = [-350_000., -50_000.].repeat(20);
    assert_eq!(read(&differences, MARGIN).admission, Admission::Refused);
}

#[test]
fn a_model_needs_an_overall_edge_and_must_clear_every_opponent_margin() {
    use svanbot_learn::margin::model_comparison;

    let gain = [8.; 30];
    let tolerable = [-2.5; 30];
    let harmful = [-3.; 30];
    let accepted = model_comparison(&[&gain, &tolerable]).unwrap();
    assert_eq!(accepted.overall.mean, 5_500.0);
    assert_eq!(accepted.rows[1].mean, -5_000.0);
    assert!(accepted.passes());
    assert!(!model_comparison(&[&gain, &harmful]).unwrap().passes());
    assert!(!model_comparison(&[&[0.; 30]]).unwrap().passes());
    assert!(!model_comparison(&[&[8.; 29]]).unwrap().passes());
    assert!(model_comparison(&[&[f64::NAN]]).is_none());
}
