//! Search-console arithmetic for SERP performance data.
//!
//! This crate does the small pile of arithmetic that sits between a Search Console
//! export and a decision: click-through rate, impression-weighted average position,
//! position movement between two periods, and a projection of clicks at a different
//! position using a CTR-by-position curve.
//!
//! # What this crate deliberately does not ship
//!
//! There is **no built-in CTR curve**. Published "average CTR by position" tables are
//! third-party estimates that differ by study, by query intent and by SERP layout, and
//! baking one in would turn somebody else's sample into this crate's constant. So
//! [`CtrCurve`] is something you construct from your *own* measured data — for example
//! the impressions and clicks your own Search Console reports at each position. A
//! projection is then a statement about your own history, not about the internet.
//!
//! Every function here is pure integer or `f64` arithmetic. No I/O, no dependencies.
//!
//! # Example
//!
//! ```
//! use serp_ctr::{ctr, weighted_average_position, PositionRow};
//!
//! let ctr = ctr(7, 1_000).unwrap();
//! assert!((ctr - 0.007).abs() < 1e-12);
//!
//! let rows = vec![
//!     PositionRow { position: 3.0, impressions: 900 },
//!     PositionRow { position: 40.0, impressions: 100 },
//! ];
//! let avg = weighted_average_position(&rows).unwrap();
//! assert!((avg - 6.7).abs() < 1e-12);
//! ```
//!
//! Built for the pipeline behind <https://toolsthatrank.com/>, which verifies a figure
//! against its source before it ships one.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::fmt;

/// Errors returned when an input cannot support the requested arithmetic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SerpError {
    /// A rate was requested from zero impressions; the ratio is undefined.
    NoImpressions,
    /// Clicks exceeded impressions, which no search-console export can legitimately report.
    ClicksExceedImpressions {
        /// The clicks that were supplied.
        clicks: u64,
        /// The impressions that were supplied.
        impressions: u64,
    },
    /// A position value was not a finite number `>= 1.0`.
    InvalidPosition,
    /// The input slice was empty, so there is nothing to aggregate.
    EmptyInput,
    /// The curve holds no observation covering the requested position.
    PositionNotInCurve(u32),
}

impl fmt::Display for SerpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SerpError::NoImpressions => write!(f, "no impressions, rate is undefined"),
            SerpError::ClicksExceedImpressions {
                clicks,
                impressions,
            } => write!(f, "clicks ({clicks}) exceed impressions ({impressions})"),
            SerpError::InvalidPosition => write!(f, "position must be a finite number >= 1.0"),
            SerpError::EmptyInput => write!(f, "input is empty"),
            SerpError::PositionNotInCurve(p) => {
                write!(f, "curve has no observation for position {p}")
            }
        }
    }
}

impl std::error::Error for SerpError {}

/// One row of a search-performance export: an average position and the impressions behind it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PositionRow {
    /// Average position for the row. Search Console reports `1.0` as the top result.
    pub position: f64,
    /// Impressions recorded at that position.
    pub impressions: u64,
}

/// Click-through rate as a fraction in `0.0..=1.0`.
///
/// # Errors
///
/// Returns [`SerpError::NoImpressions`] for zero impressions and
/// [`SerpError::ClicksExceedImpressions`] when the export is internally inconsistent.
///
/// ```
/// assert_eq!(serp_ctr::ctr(0, 500).unwrap(), 0.0);
/// assert!(serp_ctr::ctr(1, 0).is_err());
/// ```
pub fn ctr(clicks: u64, impressions: u64) -> Result<f64, SerpError> {
    if impressions == 0 {
        return Err(SerpError::NoImpressions);
    }
    if clicks > impressions {
        return Err(SerpError::ClicksExceedImpressions {
            clicks,
            impressions,
        });
    }
    Ok(clicks as f64 / impressions as f64)
}

/// Click-through rate expressed in percent, rounded to `decimals` places.
///
/// ```
/// assert_eq!(serp_ctr::ctr_percent(7, 1_000, 2).unwrap(), 0.7);
/// ```
pub fn ctr_percent(clicks: u64, impressions: u64, decimals: u32) -> Result<f64, SerpError> {
    let pct = ctr(clicks, impressions)? * 100.0;
    let factor = 10_f64.powi(decimals as i32);
    Ok((pct * factor).round() / factor)
}

/// Impression-weighted average position across many rows.
///
/// This is the only correct way to combine per-query or per-page average positions:
/// a plain mean of the position column lets a row with three impressions outvote a row
/// with thirty thousand.
///
/// # Errors
///
/// Returns [`SerpError::EmptyInput`] for an empty slice, [`SerpError::InvalidPosition`]
/// for a non-finite or sub-1.0 position, and [`SerpError::NoImpressions`] when every row
/// has zero impressions.
pub fn weighted_average_position(rows: &[PositionRow]) -> Result<f64, SerpError> {
    if rows.is_empty() {
        return Err(SerpError::EmptyInput);
    }
    let mut total_impressions: u64 = 0;
    let mut weighted_sum: f64 = 0.0;
    for row in rows {
        if !row.position.is_finite() || row.position < 1.0 {
            return Err(SerpError::InvalidPosition);
        }
        total_impressions = total_impressions.saturating_add(row.impressions);
        weighted_sum += row.position * row.impressions as f64;
    }
    if total_impressions == 0 {
        return Err(SerpError::NoImpressions);
    }
    Ok(weighted_sum / total_impressions as f64)
}

/// The SERP page a position falls on, given `per_page` results per page.
///
/// Page numbering starts at 1. A `per_page` of 0 is treated as 1.
///
/// ```
/// assert_eq!(serp_ctr::page_of_position(10.0, 10), 1);
/// assert_eq!(serp_ctr::page_of_position(11.0, 10), 2);
/// ```
pub fn page_of_position(position: f64, per_page: u32) -> u32 {
    let per_page = per_page.max(1) as f64;
    if !position.is_finite() || position < 1.0 {
        return 1;
    }
    (((position - 1.0) / per_page).floor() as u32) + 1
}

/// Change in position between two periods, positive when the position improved.
///
/// Positions count downwards (1 is best), so an improvement from 9.0 to 4.0 is
/// reported as `+5.0` rather than `-5.0`.
///
/// ```
/// assert_eq!(serp_ctr::position_gain(9.0, 4.0), 5.0);
/// assert_eq!(serp_ctr::position_gain(4.0, 9.0), -5.0);
/// ```
pub fn position_gain(before: f64, after: f64) -> f64 {
    before - after
}

/// A CTR-by-position curve built from observed data.
///
/// Positions are stored as whole-number buckets (position `3.4` falls in bucket `3`).
/// Each bucket accumulates clicks and impressions, so the curve's CTR for a bucket is
/// the real pooled rate of the data you fed it, never an assumed constant.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CtrCurve {
    buckets: Vec<(u32, u64, u64)>, // (position, clicks, impressions), sorted by position
}

impl CtrCurve {
    /// An empty curve.
    pub fn new() -> Self {
        CtrCurve {
            buckets: Vec::new(),
        }
    }

    /// Fold one observation into the curve.
    ///
    /// Non-finite or sub-1.0 positions are rejected with [`SerpError::InvalidPosition`],
    /// and inconsistent rows with [`SerpError::ClicksExceedImpressions`].
    pub fn observe(
        &mut self,
        position: f64,
        clicks: u64,
        impressions: u64,
    ) -> Result<(), SerpError> {
        if !position.is_finite() || position < 1.0 {
            return Err(SerpError::InvalidPosition);
        }
        if clicks > impressions {
            return Err(SerpError::ClicksExceedImpressions {
                clicks,
                impressions,
            });
        }
        let bucket = position.floor() as u32;
        match self.buckets.binary_search_by_key(&bucket, |b| b.0) {
            Ok(i) => {
                self.buckets[i].1 = self.buckets[i].1.saturating_add(clicks);
                self.buckets[i].2 = self.buckets[i].2.saturating_add(impressions);
            }
            Err(i) => self.buckets.insert(i, (bucket, clicks, impressions)),
        }
        Ok(())
    }

    /// Build a curve from an iterator of `(position, clicks, impressions)` observations.
    pub fn from_observations<I>(observations: I) -> Result<Self, SerpError>
    where
        I: IntoIterator<Item = (f64, u64, u64)>,
    {
        let mut curve = CtrCurve::new();
        for (position, clicks, impressions) in observations {
            curve.observe(position, clicks, impressions)?;
        }
        Ok(curve)
    }

    /// Number of populated position buckets.
    pub fn len(&self) -> usize {
        self.buckets.len()
    }

    /// Whether the curve holds no observations.
    pub fn is_empty(&self) -> bool {
        self.buckets.is_empty()
    }

    /// Total impressions observed for a bucket, or 0 if the bucket is absent.
    pub fn impressions_at(&self, position: u32) -> u64 {
        self.buckets
            .binary_search_by_key(&position, |b| b.0)
            .map(|i| self.buckets[i].2)
            .unwrap_or(0)
    }

    /// Pooled CTR observed at a whole-number position.
    ///
    /// # Errors
    ///
    /// [`SerpError::PositionNotInCurve`] when nothing was ever observed at that position,
    /// and [`SerpError::NoImpressions`] when the bucket exists but holds zero impressions.
    /// The curve never interpolates or extrapolates: an unobserved position is an error,
    /// not a guess.
    pub fn ctr_at(&self, position: u32) -> Result<f64, SerpError> {
        let i = self
            .buckets
            .binary_search_by_key(&position, |b| b.0)
            .map_err(|_| SerpError::PositionNotInCurve(position))?;
        let (_, clicks, impressions) = self.buckets[i];
        ctr(clicks, impressions)
    }

    /// Every populated bucket as `(position, ctr)`, ascending by position.
    ///
    /// Buckets holding zero impressions are skipped, because they have no rate.
    pub fn points(&self) -> Vec<(u32, f64)> {
        self.buckets
            .iter()
            .filter(|(_, _, impressions)| *impressions > 0)
            .map(|(position, clicks, impressions)| {
                (*position, *clicks as f64 / *impressions as f64)
            })
            .collect()
    }

    /// Clicks you would expect from `impressions` at `position`, using this curve.
    ///
    /// The result is `impressions * ctr_at(position)`, rounded to the nearest whole click.
    /// It is a restatement of your own observed rate at a volume you supply — it is not a
    /// forecast, and it says nothing about whether that position is reachable.
    ///
    /// # Errors
    ///
    /// Propagates the errors of [`CtrCurve::ctr_at`].
    pub fn project_clicks(&self, position: u32, impressions: u64) -> Result<u64, SerpError> {
        let rate = self.ctr_at(position)?;
        Ok((impressions as f64 * rate).round() as u64)
    }

    /// Difference in projected clicks between two positions at the same impression volume.
    ///
    /// Positive when `to` earns more than `from`.
    ///
    /// # Errors
    ///
    /// Propagates the errors of [`CtrCurve::ctr_at`] for either position.
    pub fn projected_click_delta(
        &self,
        from: u32,
        to: u32,
        impressions: u64,
    ) -> Result<i64, SerpError> {
        let before = self.project_clicks(from, impressions)? as i64;
        let after = self.project_clicks(to, impressions)? as i64;
        Ok(after - before)
    }
}

/// Aggregate totals for a set of rows: clicks, impressions and the pooled CTR.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Totals {
    /// Summed clicks.
    pub clicks: u64,
    /// Summed impressions.
    pub impressions: u64,
    /// Pooled CTR, i.e. `clicks / impressions`, not a mean of per-row CTRs.
    pub ctr: f64,
}

/// Pool `(clicks, impressions)` rows into a single [`Totals`].
///
/// Pooling is not the same as averaging the CTR column, and the difference is the
/// classic Simpson's-paradox trap in SEO reporting.
///
/// # Errors
///
/// [`SerpError::EmptyInput`] for an empty slice, [`SerpError::NoImpressions`] when the
/// totals contain no impressions.
pub fn pooled_totals(rows: &[(u64, u64)]) -> Result<Totals, SerpError> {
    if rows.is_empty() {
        return Err(SerpError::EmptyInput);
    }
    let mut clicks = 0u64;
    let mut impressions = 0u64;
    for (c, i) in rows {
        if c > i {
            return Err(SerpError::ClicksExceedImpressions {
                clicks: *c,
                impressions: *i,
            });
        }
        clicks = clicks.saturating_add(*c);
        impressions = impressions.saturating_add(*i);
    }
    let ctr = ctr(clicks, impressions)?;
    Ok(Totals {
        clicks,
        impressions,
        ctr,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "{a} != {b}");
    }

    #[test]
    fn ctr_is_a_plain_ratio() {
        approx(ctr(50, 200).unwrap(), 0.25);
        approx(ctr(0, 200).unwrap(), 0.0);
    }

    #[test]
    fn ctr_rejects_impossible_rows() {
        assert_eq!(ctr(1, 0), Err(SerpError::NoImpressions));
        assert_eq!(
            ctr(5, 4),
            Err(SerpError::ClicksExceedImpressions {
                clicks: 5,
                impressions: 4
            })
        );
    }

    #[test]
    fn ctr_percent_rounds_to_requested_places() {
        approx(ctr_percent(7, 1_000, 2).unwrap(), 0.7);
        approx(ctr_percent(1, 3, 3).unwrap(), 33.333);
        approx(ctr_percent(1, 3, 0).unwrap(), 33.0);
    }

    #[test]
    fn weighted_position_respects_impression_volume() {
        let rows = [
            PositionRow {
                position: 2.0,
                impressions: 9_000,
            },
            PositionRow {
                position: 90.0,
                impressions: 1_000,
            },
        ];
        // Plain mean would be 46.0; the weighted answer is 10.8.
        approx(weighted_average_position(&rows).unwrap(), 10.8);
    }

    #[test]
    fn weighted_position_rejects_bad_input() {
        assert_eq!(weighted_average_position(&[]), Err(SerpError::EmptyInput));
        let bad = [PositionRow {
            position: 0.5,
            impressions: 10,
        }];
        assert_eq!(
            weighted_average_position(&bad),
            Err(SerpError::InvalidPosition)
        );
        let zero = [PositionRow {
            position: 4.0,
            impressions: 0,
        }];
        assert_eq!(
            weighted_average_position(&zero),
            Err(SerpError::NoImpressions)
        );
    }

    #[test]
    fn pages_are_one_indexed() {
        assert_eq!(page_of_position(1.0, 10), 1);
        assert_eq!(page_of_position(10.9, 10), 1);
        assert_eq!(page_of_position(11.0, 10), 2);
        assert_eq!(page_of_position(21.0, 10), 3);
        assert_eq!(page_of_position(3.0, 0), 3);
    }

    #[test]
    fn position_gain_is_positive_when_climbing() {
        approx(position_gain(12.5, 3.5), 9.0);
        approx(position_gain(3.5, 12.5), -9.0);
    }

    #[test]
    fn curve_pools_observations_into_buckets() {
        let mut curve = CtrCurve::new();
        curve.observe(1.2, 30, 100).unwrap();
        curve.observe(1.9, 20, 100).unwrap();
        curve.observe(8.0, 1, 100).unwrap();
        assert_eq!(curve.len(), 2);
        approx(curve.ctr_at(1).unwrap(), 0.25);
        approx(curve.ctr_at(8).unwrap(), 0.01);
        assert_eq!(curve.impressions_at(1), 200);
        assert_eq!(curve.impressions_at(4), 0);
    }

    #[test]
    fn curve_refuses_to_invent_unobserved_positions() {
        let curve = CtrCurve::from_observations([(1.0, 10, 100)]).unwrap();
        assert_eq!(curve.ctr_at(5), Err(SerpError::PositionNotInCurve(5)));
        assert!(curve.project_clicks(5, 1_000).is_err());
    }

    #[test]
    fn curve_rejects_invalid_observations() {
        let mut curve = CtrCurve::new();
        assert_eq!(curve.observe(0.9, 1, 10), Err(SerpError::InvalidPosition));
        assert_eq!(curve.observe(f64::NAN, 1, 10), Err(SerpError::InvalidPosition));
        assert!(curve.observe(2.0, 11, 10).is_err());
        assert!(curve.is_empty());
    }

    #[test]
    fn projection_restates_the_observed_rate() {
        let curve =
            CtrCurve::from_observations([(3.0, 60, 1_000), (9.0, 10, 1_000)]).unwrap();
        assert_eq!(curve.project_clicks(3, 5_000).unwrap(), 300);
        assert_eq!(curve.project_clicks(9, 5_000).unwrap(), 50);
        assert_eq!(curve.projected_click_delta(9, 3, 5_000).unwrap(), 250);
        assert_eq!(curve.projected_click_delta(3, 9, 5_000).unwrap(), -250);
    }

    #[test]
    fn curve_points_are_sorted_and_skip_empty_buckets() {
        let mut curve = CtrCurve::new();
        curve.observe(9.0, 1, 10).unwrap();
        curve.observe(2.0, 5, 10).unwrap();
        curve.observe(5.0, 0, 0).unwrap();
        let points = curve.points();
        assert_eq!(points.len(), 2);
        assert_eq!(points[0].0, 2);
        assert_eq!(points[1].0, 9);
    }

    #[test]
    fn pooled_totals_differ_from_a_mean_of_rates() {
        let rows = [(1u64, 10u64), (100, 10_000)];
        let totals = pooled_totals(&rows).unwrap();
        assert_eq!(totals.clicks, 101);
        assert_eq!(totals.impressions, 10_010);
        approx(totals.ctr, 101.0 / 10_010.0);
        // The mean of the two row CTRs would be 0.055, five times the pooled rate.
        assert!(totals.ctr < 0.055);
    }

    #[test]
    fn pooled_totals_rejects_bad_input() {
        assert_eq!(pooled_totals(&[]), Err(SerpError::EmptyInput));
        assert!(pooled_totals(&[(5, 1)]).is_err());
        assert_eq!(pooled_totals(&[(0, 0)]), Err(SerpError::NoImpressions));
    }

    #[test]
    fn errors_display_readably() {
        assert_eq!(
            SerpError::PositionNotInCurve(7).to_string(),
            "curve has no observation for position 7"
        );
    }
}
