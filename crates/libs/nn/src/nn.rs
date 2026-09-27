//! Small dense MLP (ReLU hidden layers, softmax output) with Adam training.
//! Sized for CPU: a few thousand parameters, microseconds per forward pass.

use serde::{Deserialize, Serialize};
use sv10_rng::RngExt;
use sv10_rng::SeedableRng;
use sv10_rng::rngs::SmallRng;

/// One dense layer: `outputs × inputs` weights (row per output) and `outputs` biases.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Layer {
    /// Input width.
    pub inputs: usize,
    /// Output width.
    pub outputs: usize,
    /// Weights, row-major: `w[o * inputs + i]`.
    pub w: Vec<f32>,
    /// Biases per output.
    pub b: Vec<f32>,
}

/// A multilayer perceptron with input standardization; serialized into the store as the response model.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Mlp {
    /// Layers in order; ReLU between them, softmax after the last.
    pub layers: Vec<Layer>,
    /// Per-feature standardization learned from the training set.
    pub mean: Vec<f32>,
    /// Per-feature standard deviation paired with `mean` (1 for untrained nets).
    pub std: Vec<f32>,
}

impl Mlp {
    /// A He-initialized net with layer widths `sizes` (input first), seeded for reproducibility.
    pub fn new(sizes: &[usize], seed: u64) -> Mlp {
        let mut rng = SmallRng::seed_from_u64(seed);
        let layers = sizes
            .windows(2)
            .map(|w| {
                let scale = (2.0 / w[0] as f32).sqrt();
                Layer {
                    inputs: w[0],
                    outputs: w[1],
                    w: (0..w[0] * w[1]).map(|_| (rng.random::<f32>() * 2.0 - 1.0) * scale).collect(),
                    b: vec![0.0; w[1]],
                }
            })
            .collect();
        Mlp { layers, mean: vec![0.0; sizes[0]], std: vec![1.0; sizes[0]] }
    }

    /// Number of input features.
    pub fn input_size(&self) -> usize {
        self.layers[0].inputs
    }

    fn normalize(&self, x: &[f32]) -> Vec<f32> {
        x.iter().zip(self.mean.iter().zip(self.std.iter())).map(|(v, (m, s))| (v - m) / s).collect()
    }

    /// Class probabilities, with `mask[i] == false` classes forced to zero.
    pub fn predict(&self, x: &[f32], mask: &[bool]) -> Vec<f32> {
        let mut a = self.normalize(x);
        for (li, l) in self.layers.iter().enumerate() {
            let mut z = l.b.clone();
            for o in 0..l.outputs {
                let row = &l.w[o * l.inputs..(o + 1) * l.inputs];
                z[o] += row.iter().zip(a.iter()).map(|(w, v)| w * v).sum::<f32>();
            }
            if li + 1 < self.layers.len() {
                for v in z.iter_mut() {
                    *v = v.max(0.0);
                }
            }
            a = z;
        }
        softmax_masked(&a, mask)
    }
}

/// Softmax over `z` with masked-out classes at probability zero.
pub fn softmax_masked(z: &[f32], mask: &[bool]) -> Vec<f32> {
    let m = z.iter().zip(mask).filter(|(_, k)| **k).map(|(v, _)| *v).fold(f32::MIN, f32::max);
    let exps: Vec<f32> = z.iter().zip(mask).map(|(v, k)| if *k { (v - m).exp() } else { 0.0 }).collect();
    let s: f32 = exps.iter().sum();
    exps.iter().map(|e| e / s.max(1e-12)).collect()
}

/// One training example.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sample {
    /// Raw (unstandardized) features.
    pub x: Vec<f32>,
    /// Which classes were legal for this example.
    pub mask: Vec<bool>,
    /// Index of the class that happened.
    pub label: usize,
    /// Loss weight (every current producer uses 1.0).
    pub weight: f32,
}

/// Negative log-likelihood of `label` under `probs` (probability floored at 1e-6).
pub fn log_loss(probs: &[f32], label: usize) -> f64 {
    -(probs[label].max(1e-6) as f64).ln()
}

/// Training hyperparameters (defaults: 12 epochs, batch 128, lr 2e-3, L2 1e-5).
pub struct TrainConfig {
    /// Passes over the data.
    pub epochs: usize,
    /// Mini-batch size.
    pub batch: usize,
    /// Adam learning rate.
    pub lr: f32,
    /// L2 weight decay.
    pub l2: f32,
    /// Shuffle seed.
    pub seed: u64,
}

impl Default for TrainConfig {
    fn default() -> Self {
        TrainConfig { epochs: 12, batch: 128, lr: 2e-3, l2: 1e-5, seed: 1 }
    }
}

/// Train in place with Adam on weighted masked cross-entropy.
pub fn train(net: &mut Mlp, data: &[Sample], cfg: &TrainConfig) {
    if data.is_empty() {
        return;
    }
    let d = net.input_size();
    let n = data.len() as f32;
    for j in 0..d {
        let mean = data.iter().map(|s| s.x[j]).sum::<f32>() / n;
        let var = data.iter().map(|s| (s.x[j] - mean).powi(2)).sum::<f32>() / n;
        net.mean[j] = mean;
        net.std[j] = var.sqrt().max(1e-3);
    }
    let normalized: Vec<Vec<f32>> = data.iter().map(|s| net.normalize(&s.x)).collect();
    let mut m_w: Vec<Vec<f32>> = net.layers.iter().map(|l| vec![0.0; l.w.len()]).collect();
    let mut v_w = m_w.clone();
    let mut m_b: Vec<Vec<f32>> = net.layers.iter().map(|l| vec![0.0; l.b.len()]).collect();
    let mut v_b = m_b.clone();
    let (b1, b2, eps) = (0.9f32, 0.999f32, 1e-8f32);
    let mut step = 0i32;
    let mut order: Vec<usize> = (0..data.len()).collect();
    let mut rng = SmallRng::seed_from_u64(cfg.seed);
    for _epoch in 0..cfg.epochs {
        for i in (1..order.len()).rev() {
            let j = rng.random_range(0..=i);
            order.swap(i, j);
        }
        for chunk in order.chunks(cfg.batch) {
            let mut g_w: Vec<Vec<f32>> = net.layers.iter().map(|l| vec![0.0; l.w.len()]).collect();
            let mut g_b: Vec<Vec<f32>> = net.layers.iter().map(|l| vec![0.0; l.b.len()]).collect();
            let mut wsum = 0.0f32;
            for &idx in chunk {
                let s = &data[idx];
                wsum += s.weight;
                // Forward, keeping activations.
                let mut acts: Vec<Vec<f32>> = vec![normalized[idx].clone()];
                for (li, l) in net.layers.iter().enumerate() {
                    let a = acts.last().unwrap();
                    let mut z = l.b.clone();
                    for o in 0..l.outputs {
                        let row = &l.w[o * l.inputs..(o + 1) * l.inputs];
                        z[o] += row.iter().zip(a.iter()).map(|(w, v)| w * v).sum::<f32>();
                    }
                    if li + 1 < net.layers.len() {
                        for v in z.iter_mut() {
                            *v = v.max(0.0);
                        }
                    }
                    acts.push(z);
                }
                let probs = softmax_masked(acts.last().unwrap(), &s.mask);
                let mut delta: Vec<f32> =
                    probs.iter().enumerate().map(|(k, p)| (p - if k == s.label { 1.0 } else { 0.0 }) * s.weight).collect();
                for (k, keep) in s.mask.iter().enumerate() {
                    if !keep {
                        delta[k] = 0.0;
                    }
                }
                for li in (0..net.layers.len()).rev() {
                    let l = &net.layers[li];
                    let input = &acts[li];
                    for o in 0..l.outputs {
                        g_b[li][o] += delta[o];
                        let row = &mut g_w[li][o * l.inputs..(o + 1) * l.inputs];
                        for (g, v) in row.iter_mut().zip(input.iter()) {
                            *g += delta[o] * v;
                        }
                    }
                    if li > 0 {
                        let mut prev = vec![0.0f32; l.inputs];
                        for o in 0..l.outputs {
                            let row = &l.w[o * l.inputs..(o + 1) * l.inputs];
                            for (p, w) in prev.iter_mut().zip(row.iter()) {
                                *p += delta[o] * w;
                            }
                        }
                        for (p, a) in prev.iter_mut().zip(acts[li].iter()) {
                            if *a <= 0.0 {
                                *p = 0.0;
                            }
                        }
                        delta = prev;
                    }
                }
            }
            step += 1;
            let bc1 = 1.0 - b1.powi(step);
            let bc2 = 1.0 - b2.powi(step);
            let scale = 1.0 / wsum.max(1e-6);
            for li in 0..net.layers.len() {
                let l = &mut net.layers[li];
                for k in 0..l.w.len() {
                    let g = g_w[li][k] * scale + cfg.l2 * l.w[k];
                    m_w[li][k] = b1 * m_w[li][k] + (1.0 - b1) * g;
                    v_w[li][k] = b2 * v_w[li][k] + (1.0 - b2) * g * g;
                    l.w[k] -= cfg.lr * (m_w[li][k] / bc1) / ((v_w[li][k] / bc2).sqrt() + eps);
                }
                for k in 0..l.b.len() {
                    let g = g_b[li][k] * scale;
                    m_b[li][k] = b1 * m_b[li][k] + (1.0 - b1) * g;
                    v_b[li][k] = b2 * v_b[li][k] + (1.0 - b2) * g * g;
                    l.b[k] -= cfg.lr * (m_b[li][k] / bc1) / ((v_b[li][k] / bc2).sqrt() + eps);
                }
            }
        }
    }
}

/// Weighted mean log-loss of `net` over `data` (the held-out gate's metric).
pub fn mean_log_loss(net: &Mlp, data: &[Sample]) -> f64 {
    let (mut sum, mut w) = (0.0, 0.0);
    for s in data {
        sum += log_loss(&net.predict(&s.x, &s.mask), s.label) * s.weight as f64;
        w += s.weight as f64;
    }
    sum / w.max(1e-9)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learns_a_nonlinear_rule() {
        // Class depends on the sign of x0*x1 (XOR-like), class 2 masked out half the time.
        let mut rng = SmallRng::seed_from_u64(9);
        let data: Vec<Sample> = (0..4000)
            .map(|_| {
                let a = rng.random::<f32>() * 2.0 - 1.0;
                let b = rng.random::<f32>() * 2.0 - 1.0;
                let label = if a * b > 0.0 { 0 } else { 1 };
                Sample { x: vec![a, b, rng.random::<f32>()], mask: vec![true, true, rng.random::<bool>()], label, weight: 1.0 }
            })
            .collect();
        let mut net = Mlp::new(&[3, 16, 16, 3], 3);
        let before = mean_log_loss(&net, &data);
        train(&mut net, &data, &TrainConfig { epochs: 40, ..Default::default() });
        let after = mean_log_loss(&net, &data);
        assert!(after < 0.25 && after < before, "before {before} after {after}");
        let p = net.predict(&[0.5, 0.5, 0.3], &[true, true, false]);
        assert!(p[0] > 0.8 && p[2] == 0.0, "{p:?}");
    }
}
