//! The caller-count mixture for a raise that three or more players could call (#746 layer 1). The
//! default pricing collapses every multiway call into "the top two continuers"; with `caller_mix` on,
//! the number of callers is a mixture over one, two and three-plus, and three-plus is priced against
//! the top three. Callers are modelled as independent, as the one/two split already was.

use super::Rng;

/// Probability that exactly one of the `active` players continues, each with probability `conts[i]`
/// (the same arithmetic, in the same order, the pricing always used).
pub(super) fn exactly_one(active: &[usize], conts: &[f64]) -> f64 {
    let mut total = 0.0;
    for &i in active {
        let mut p = conts[i];
        for &j in active {
            if j != i {
                p *= 1.0 - conts[j];
            }
        }
        total += p;
    }
    total
}

/// Mixed value and equity of the called branch, `Ok(None)` when there is no three-plus world to add,
/// `Err(())` when the three-caller equity cannot be measured (the caller refuses the spot, as for any
/// unmeasured price).
///
/// `active` are the players who can still act, `order` them by continuation probability, `money` the
/// continuation probabilities and what each puts in, `shares` the called probability and the
/// exactly-one-caller share, `equities` hero's equity against the top one and top two, and `pricing`
/// the pot after our raise, the realization factor and our added chips.
#[allow(clippy::too_many_arguments, clippy::result_unit_err)]
pub(super) fn mix<R: Rng>(
    active: &[usize],
    order: &[usize],
    money: (&[f64], &[f64]),
    shares: (f64, f64),
    equities: [f64; 2],
    pricing: (f64, f64, f64),
    vs_continuing: &dyn Fn(&[usize], &mut R) -> Option<f64>,
    rng: &mut R,
) -> Result<Option<(f64, f64)>, ()> {
    let ((conts, costs), (called, p_one), (pot_add, r, add)) = (money, shares, pricing);
    if order.len() < 3 {
        return Ok(None);
    }
    let mut p_exactly_two = 0.0;
    for (n, &i) in active.iter().enumerate() {
        for &j in &active[n + 1..] {
            let others: f64 = active.iter().filter(|&&k| k != i && k != j).map(|&k| 1.0 - conts[k]).product();
            p_exactly_two += conts[i] * conts[j] * others;
        }
    }
    let p_two = (p_exactly_two / called).clamp(0.0, 1.0 - p_one);
    let p_three = 1.0 - p_one - p_two;
    if p_three <= 1e-9 {
        return Ok(None);
    }
    let top = [order[0], order[1], order[2]];
    let eq3 = vs_continuing(&top, rng).ok_or(())?;
    let value = |eq: f64, callers: &[usize]| eq * (pot_add + callers.iter().map(|&c| costs[c]).sum::<f64>()) * r - add;
    let [eq1, eq2] = equities;
    let called_value = p_one * value(eq1, &top[..1]) + p_two * value(eq2, &top[..2]) + p_three * value(eq3, &top);
    Ok(Some((called_value, p_one * eq1 + p_two * eq2 + p_three * eq3)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sv10_rng::SeedableRng;
    use sv10_rng::rngs::SmallRng;

    #[test]
    fn exactly_one_is_the_independent_probability_and_the_mixture_adds_a_three_plus_world() {
        let (active, conts) = ([0usize, 1, 2], [0.5, 0.4, 0.3]);
        let brute = 0.5 * 0.6 * 0.7 + 0.5 * 0.4 * 0.7 + 0.5 * 0.6 * 0.3;
        assert!((exactly_one(&active, &conts) - brute).abs() < 1e-12);
        let costs = [100.0; 3];
        let called = 1.0 - 0.5 * 0.6 * 0.7;
        let p_one = exactly_one(&active, &conts) / called;
        // Hero's equity falls with every extra caller, so the three-plus world pulls the value down.
        let vs = |callers: &[usize], _: &mut SmallRng| Some([0.0, 0.6, 0.5, 0.35][callers.len()]);
        let mut rng = SmallRng::seed_from_u64(1);
        let (value, eq) =
            mix(&active, &[0, 1, 2], (&conts, &costs), (called, p_one), [0.6, 0.5], (300.0, 1.0, 50.0), &vs, &mut rng).unwrap().unwrap();
        let two_only = p_one * (0.6 * 400.0 - 50.0) + (1.0 - p_one) * (0.5 * 500.0 - 50.0);
        assert!(value < two_only && eq < p_one * 0.6 + (1.0 - p_one) * 0.5, "{value} {two_only} {eq}");
        // Two players cannot make a three-plus world; an unmeasurable three-caller equity refuses.
        let none = mix(&[0, 1], &[0, 1], (&conts, &costs), (0.7, 0.5), [0.6, 0.5], (300.0, 1.0, 50.0), &vs, &mut rng);
        assert_eq!(none, Ok(None));
        let refuse = |_: &[usize], _: &mut SmallRng| None;
        assert_eq!(mix(&active, &[0, 1, 2], (&conts, &costs), (called, p_one), [0.6, 0.5], (300.0, 1.0, 50.0), &refuse, &mut rng), Err(()));
    }
}
