//! A source of unique, strictly increasing, positive numbers.

use vstd::prelude::*;

verus! {

/// Hands out 1, 2, 3, … with no gaps and no repeats. Topology uses one for identity
/// epochs and another for resync generations.
///
/// **Proven:**
/// - Every number returned is positive.
/// - Every number returned is exactly one more than the previous one, so it is greater
///   than every number returned before it.
/// - Once `u64::MAX - 1` numbers have been handed out, `allocate` returns `None`
///   forever, instead of wrapping around or repeating a number.
///
/// The type invariant (the next number is at least 1) holds for every value, because
/// the only constructor establishes it and `allocate` preserves it. Callers therefore
/// have nothing to prove.
#[derive(Debug)]
pub struct MonotonicCounter {
    next: u64,
}

impl MonotonicCounter {
    #[verifier::type_invariant]
    spec fn starts_at_one(self) -> bool {
        self.next >= 1
    }

    /// The number the next successful `allocate` will return.
    pub closed spec fn peek(self) -> u64 {
        self.next
    }

    /// A counter that has handed out nothing yet. Its first number is 1.
    pub fn new() -> (counter: Self)
        ensures
            counter.peek() == 1,
    {
        MonotonicCounter { next: 1 }
    }

    /// A counter that resumes after `last`, the highest number handed out by an
    /// earlier counter (for example, the last epoch stored before a restart). `None` if
    /// `last` is `u64::MAX`, because nothing would be left to hand out.
    pub fn resuming_after(last: u64) -> (counter: Option<Self>)
        ensures
            match counter {
                Some(c) => c.peek() == last + 1,
                None => last == u64::MAX,
            },
    {
        if last == u64::MAX {
            None
        } else {
            Some(MonotonicCounter { next: last + 1 })
        }
    }

    /// The next number, or `None` once the `u64` space is used up.
    ///
    /// At one allocation per nanosecond, the space lasts 584 years. Callers may treat
    /// `None` as unreachable, but this function never pretends it is.
    pub fn allocate(&mut self) -> (number: Option<u64>)
        ensures
            match number {
                Some(n) => n >= 1 && n == old(self).peek() && final(self).peek() == n + 1,
                None => old(self).peek() == u64::MAX && final(self).peek() == u64::MAX,
            },
    {
        proof {
            use_type_invariant(&*self);
        }
        if self.next == u64::MAX {
            None
        } else {
            let n = self.next;
            self.next = n + 1;
            Some(n)
        }
    }
}

impl Default for MonotonicCounter {
    fn default() -> Self {
        Self::new()
    }
}

} // verus!
