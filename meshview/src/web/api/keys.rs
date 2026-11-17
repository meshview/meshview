// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! The public keys a node's node info packets carry, told apart.
//!
//! A packet damaged on the air now and then gets through with a few bits of
//! its key flipped: the encryption flips the same bits of what it decrypts to,
//! so the key comes out almost the one sent. Two keys made apart, at random,
//! differ in about half their 256 bits; a damaged copy, in a few. So keys
//! closer than [`SAME_KEY_BITS`] are taken as the same, the one heard most
//! often being the right one.

/// The length of a Meshtastic public key.
pub const PUBLIC_KEY_LEN: usize = 32;

/// Fewer bits than this differing, two keys are one, damaged. Copies seen
/// damaged differ in under 30 bits, keys made apart in over a hundred.
const SAME_KEY_BITS: u32 = 64;

fn differing_bits(a: &[u8], b: &[u8]) -> u32 {
    a.iter().zip(b).map(|(x, y)| (x ^ y).count_ones()).sum()
}

/// The keys these are, latest first as they come: each the copy heard most
/// often of those within a few bits of it. Only whole keys count.
pub fn tell_apart<'a>(keys: impl IntoIterator<Item = &'a [u8]>) -> Vec<&'a [u8]> {
    // Each copy, with how often it was heard, in the order first heard.
    let mut copies: Vec<(&[u8], usize)> = Vec::new();
    for key in keys.into_iter().filter(|key| key.len() == PUBLIC_KEY_LEN) {
        match copies.iter_mut().find(|(copy, _)| *copy == key) {
            Some((_, count)) => *count += 1,
            None => copies.push((key, 1)),
        }
    }
    // The most heard first, so that each kind is told by its right copy.
    let mut by_count: Vec<usize> = (0..copies.len()).collect();
    by_count.sort_by_key(|&index| std::cmp::Reverse(copies[index].1));
    let mut chosen: Vec<usize> = Vec::new();
    for index in by_count {
        let key = copies[index].0;
        if !chosen
            .iter()
            .any(|&other| differing_bits(copies[other].0, key) < SAME_KEY_BITS)
        {
            chosen.push(index);
        }
    }
    chosen.sort_unstable();
    chosen.into_iter().map(|index| copies[index].0).collect()
}

/// Which of `keys`, as [`tell_apart`] gives them, this one is a copy of.
pub fn right_copy<'a>(keys: &[&'a [u8]], key: &[u8]) -> Option<&'a [u8]> {
    keys.iter()
        .copied()
        .find(|right| differing_bits(right, key) < SAME_KEY_BITS)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A key with these of its bits flipped.
    fn damaged(key: &[u8; 32], bits: &[usize]) -> [u8; 32] {
        let mut copy = *key;
        for &bit in bits {
            copy[bit / 8] ^= 1 << (bit % 8);
        }
        copy
    }

    #[test]
    fn a_copy_damaged_on_the_air_is_the_same_key() {
        let right = [0x5a; 32];
        let bad = damaged(&right, &[3, 40, 41, 200]);
        // Latest first: the damaged copy is the latest, but heard once.
        let keys = tell_apart([&bad[..], &right, &right]);
        assert_eq!(keys, [&right[..]]);
        assert_eq!(right_copy(&keys, &bad), Some(&right[..]));
    }

    #[test]
    fn keys_made_apart_are_told_apart() {
        let old = [0x00; 32];
        let new = [0x0f; 32];
        let bad = damaged(&old, &[7]);
        let keys = tell_apart([&new[..], &new, &old, &bad]);
        assert_eq!(keys, [&new[..], &old]);
        // A key cut short is no key.
        assert_eq!(tell_apart([&new[..16]]), Vec::<&[u8]>::new());
    }
}
