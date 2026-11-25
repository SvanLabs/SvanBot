#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use svanbot_learn::calibration::Point;
use svanbot_runtime::fleet::Fleet;

#[test]
fn a_calibration_point_is_kept_once_a_probe_and_the_newest_fifty() {
    let path = std::env::temp_dir().join(format!("svanbot-calibration-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    let fleet = Fleet::in_dir(path);
    assert!(fleet.calibration().unwrap().is_empty());
    for id in 0..52 {
        let point = Point {
            sim: id as f64,
            live: 2.,
            radius: 1.,
        };
        fleet
            .record_calibration(id, &point, "cleared", id as i64)
            .unwrap();
    }
    let replacement = Point {
        sim: 50.,
        live: 3.,
        radius: 2.,
    };
    fleet
        .record_calibration(50, &replacement, "harmful", 52)
        .unwrap();
    let retained = fleet.calibration().unwrap();
    assert_eq!(retained.len(), 50);
    assert_eq!(retained[0], replacement);
    assert_eq!(retained[1].sim, 51.);
    assert_eq!(retained.last().unwrap().sim, 2.);
    assert_eq!(retained.iter().filter(|point| point.sim == 50.).count(), 1);
}
