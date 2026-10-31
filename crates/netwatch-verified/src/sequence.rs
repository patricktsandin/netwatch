//! Probe sequence numbers: an index's low bits, and why reuse is safe (ADR-0023).

use vstd::prelude::*;

verus! {

/// The on-the-wire sequence number of the probe with stream index `index`, in a
/// sequence space of `space` numbers (2^16 for ICMP).
///
/// **Proven:** the result is in range, and it is exactly `index mod space`, so two
/// probes share a sequence number only if their indices are a multiple of `space`
/// apart (see [`indices_within_one_space_have_distinct_sequences`]).
pub fn sequence_of(index: u64, space: u64) -> (sequence: u64)
    requires
        space > 0,
    ensures
        sequence < space,
        sequence == index % space,
{
    index % space
}

/// **Proven:** two probes less than one sequence space apart never share a sequence
/// number. Together with `StreamConfig`'s rule (timeout < space × cadence, so fewer
/// than `space` probes are ever in flight at once), this means a sequence number is
/// never reused while its probe is still in flight.
pub proof fn indices_within_one_space_have_distinct_sequences(i: u64, j: u64, space: u64)
    requires
        space > 0,
        i < j,
        j - i < space,
    ensures
        i % space != j % space,
{
    let (a, b, s) = (i as int, j as int, space as int);
    vstd::arithmetic::div_mod::lemma_fundamental_div_mod(a, s);
    vstd::arithmetic::div_mod::lemma_fundamental_div_mod(b, s);
    vstd::arithmetic::div_mod::lemma_mod_bound(a, s);
    vstd::arithmetic::div_mod::lemma_mod_bound(b, s);
    if a % s == b % s {
        // Then b - a == s * (b / s - a / s), a positive multiple of s, so at least s.
        assert(b - a == s * (b / s - a / s)) by (nonlinear_arith)
            requires
                a == s * (a / s) + a % s,
                b == s * (b / s) + b % s,
                a % s == b % s;
        assert(b / s - a / s >= 1) by (nonlinear_arith)
            requires
                b - a == s * (b / s - a / s),
                b - a > 0,
                s > 0;
        assert(b - a >= s) by (nonlinear_arith)
            requires
                b - a == s * (b / s - a / s),
                b / s - a / s >= 1,
                s > 0;
    }
}

} // verus!
