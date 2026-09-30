//! Held-out paired likelihood gains, clustered by showdown hand.

#[derive(serde::Serialize)]
pub struct Evidence {
    pub samples: usize,
    pub hands: usize,
    pub mean_nats: f64,
    pub se_nats: f64,
    pub lower_95_nats: f64,
}

impl Evidence {
    pub fn active(&self) -> bool {
        self.lower_95_nats > 0.005
    }
}

pub fn evidence(gains: &[Vec<f64>]) -> Evidence {
    let hands = gains.len();
    let samples = gains.iter().map(Vec::len).sum::<usize>();
    let mean_nats = gains.iter().flatten().sum::<f64>() / samples.max(1) as f64;
    let se_nats = if hands < 2 || samples == 0 {
        f64::INFINITY
    } else {
        let squared = gains.iter().map(|hand| hand.iter().map(|gain| gain - mean_nats).sum::<f64>().powi(2)).sum::<f64>();
        (hands as f64 / (hands - 1) as f64 * squared / (samples as f64).powi(2)).sqrt()
    };
    Evidence { samples, hands, mean_nats, se_nats, lower_95_nats: mean_nats - 1.96 * se_nats }
}

/// Put whole hands on either side of the sample-count target, never split shown seats of a hand.
pub fn split(counts: &[usize]) -> (usize, usize) {
    let target = counts.iter().sum::<usize>() * 3 / 4;
    let mut samples = 0;
    let mut hands = 0;
    for count in counts {
        if samples + count > target {
            break;
        }
        samples += count;
        hands += 1;
    }
    (samples, hands)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noisy_point_gain_cannot_activate_a_range_fit() {
        let e = evidence(&[vec![0.106], vec![-0.094]]);
        assert!(e.mean_nats > 0.005);
        assert!(!e.active());
        assert!(!evidence(&[vec![0.02]]).active());
        assert!(evidence(&vec![vec![0.01]; 100]).active());
    }

    #[test]
    fn shown_seats_share_one_cluster_and_one_split_side() {
        let e = evidence(&[vec![0.1, 0.1], vec![-0.1, -0.1]]);
        assert_eq!((e.samples, e.hands), (4, 2));
        assert!((e.se_nats - 0.1).abs() < 1e-12);
        assert_eq!(split(&[2, 3, 2, 3]), (7, 3));
        assert_eq!(split(&[2, 2, 3, 3]), (7, 3));
        assert_eq!(split(&[3, 3, 3, 3]), (9, 3));
    }
}
