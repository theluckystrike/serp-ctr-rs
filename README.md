# serp-ctr

Search-console arithmetic in safe, dependency-free Rust: click-through rate,
impression-weighted average position, SERP page bucketing, pooled totals, and click
projection against a CTR-by-position curve.

## Why there is no built-in CTR table

Published "average CTR by position" tables are third-party estimates that disagree with
each other and vary by query intent and SERP layout. Baking one in would turn somebody
else's sample into this crate's constant. `CtrCurve` is therefore built from *your*
observations, and it returns an error for a position it has never seen rather than
interpolating one.

```rust
use serp_ctr::{ctr_percent, CtrCurve, PositionRow, weighted_average_position};

// 7 clicks on 1,000 impressions
assert_eq!(ctr_percent(7, 1_000, 2).unwrap(), 0.7);

// Impression-weighted, not a mean of the position column
let rows = [
    PositionRow { position: 2.0, impressions: 9_000 },
    PositionRow { position: 90.0, impressions: 1_000 },
];
assert!((weighted_average_position(&rows).unwrap() - 10.8).abs() < 1e-9);

// Curve built from measured rows, then used to restate them at a new volume
let curve = CtrCurve::from_observations([(3.0, 60, 1_000), (9.0, 10, 1_000)]).unwrap();
assert_eq!(curve.project_clicks(3, 5_000).unwrap(), 300);
assert!(curve.ctr_at(5).is_err()); // never observed, so never guessed
```

## Install

```toml
[dependencies]
serp-ctr = "0.1"
```

`#![forbid(unsafe_code)]`, no dependencies, MSRV 1.63.

## Licence

MIT OR Apache-2.0.

Written for the SEO pipeline at <https://toolsthatrank.com/>.
