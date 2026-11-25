//! Report the agreement between paired estimates and completed live probes.

use std::process::ExitCode;

use svanbot_learn::calibration::slope;
use svanbot_runtime::fleet::Fleet;

pub(crate) fn run() -> ExitCode {
    let fleet = Fleet::in_dir(crate::fleet_dir());
    let points = match fleet.calibration() {
        Ok(points) => points,
        Err(error) => {
            eprintln!("svanbot calibration: {error}");
            return ExitCode::from(1);
        }
    };
    for point in &points {
        println!(
            "sim {:+.1}, live {:+.1} ±{:.1} bb/100",
            point.sim, point.live, point.radius
        );
    }
    if let Some(fit) = slope(&points) {
        println!(
            "calibration: slope {:.1} ±{:.1} over {} probes",
            fit.value, fit.radius, fit.points
        );
    } else if points.len() < 2 {
        println!("calibration: fewer than two probes");
    } else {
        println!(
            "calibration: slope unavailable over {} probes",
            points.len()
        );
    }
    ExitCode::SUCCESS
}
