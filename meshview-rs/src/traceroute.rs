// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! Paths out of traceroutes, as the Python meshview's `traceroute.py` makes
//! them.
//!
//! A traceroute's answer carries its endpoints the other way round from the
//! path traced: `to` is the node that started it and `from` the node that
//! answered. Only the answer's direction is affected, so both are handled here
//! rather than wherever a path is drawn.

/// What the firmware puts in a route for a hop it could not tell.
pub const UNKNOWN_HOP: i64 = 0xffff_ffff;

/// The path towards the traced node, from the one that started the traceroute.
pub fn forward_path(
    from: Option<i64>,
    to: Option<i64>,
    hops: &[i64],
    done: bool,
    gateway: Option<i64>,
) -> Vec<i64> {
    if done {
        return to
            .into_iter()
            .chain(hops.iter().copied())
            .chain(from)
            .collect();
    }
    let mut path: Vec<i64> = from.into_iter().chain(hops.iter().copied()).collect();
    // Some nodes add themselves to the route before uplinking.
    if let Some(gateway) = gateway
        && path.last() != Some(&gateway)
    {
        path.push(gateway);
    }
    path
}

/// The path back from the traced node, from the one that answered; reaching
/// the node that started it only when the answer was seen arriving there.
pub fn return_path(from: Option<i64>, to: Option<i64>, hops: &[i64], complete: bool) -> Vec<i64> {
    let mut path: Vec<i64> = from.into_iter().chain(hops.iter().copied()).collect();
    if complete && let Some(to) = to {
        path.push(to);
    }
    path
}

/// The traceroute's (initiator, target), whichever way the packet went.
pub fn endpoints(from: Option<i64>, to: Option<i64>, done: bool) -> (Option<i64>, Option<i64>) {
    if done { (to, from) } else { (from, to) }
}

/// Consecutive pairs of a path, leaving out the links through a hop that could
/// not be told.
pub fn edges(path: &[i64]) -> impl Iterator<Item = (i64, i64)> + '_ {
    path.windows(2)
        .map(|pair| (pair[0], pair[1]))
        .filter(|(a, b)| *a != UNKNOWN_HOP && *b != UNKNOWN_HOP)
}

/// Whether the answer was seen after it reached the node that started the
/// traceroute: relays add their id and their SNR, but the last node only its
/// SNR, so one SNR more than hops means it got home.
pub fn return_is_complete(snr_back: usize, route_back: usize) -> bool {
    snr_back == route_back + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_run_from_the_initiator() {
        assert_eq!(
            forward_path(Some(222), Some(111), &[444, 555], true, None),
            [111, 444, 555, 222]
        );
        assert_eq!(
            forward_path(Some(111), Some(222), &[444], false, Some(333)),
            [111, 444, 333]
        );
        assert_eq!(
            forward_path(Some(111), Some(222), &[333], false, Some(333)),
            [111, 333]
        );
        assert_eq!(
            return_path(Some(222), Some(111), &[555, 444], true),
            [222, 555, 444, 111]
        );
        assert_eq!(return_path(Some(222), Some(111), &[555], false), [222, 555]);
        assert_eq!(
            endpoints(Some(222), Some(111), true),
            (Some(111), Some(222))
        );
    }

    #[test]
    fn unknown_hops_break_the_links_through_them() {
        let path = [1, UNKNOWN_HOP, 2, 3];
        assert_eq!(edges(&path).collect::<Vec<_>>(), [(2, 3)]);
    }
}
