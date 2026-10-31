//! A probe stream's timing, validated so that sequence reuse is always safe.

use netwatch_shared::uom::si::f64::Time;

/// How many distinct sequence numbers a stream has before it wraps around.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SequenceSpace(u32);

impl SequenceSpace {
    /// ICMP and ICMPv6 echo: a 16-bit sequence field.
    pub const ICMP: SequenceSpace = SequenceSpace(1 << 16);

    /// A space of `size` sequence numbers. Spaces other than [`Self::ICMP`] exist so
    /// that the spec's tiny space can be exercised; `None` if `size` is zero or
    /// larger than 2^16.
    pub const fn new(size: u32) -> Option<Self> {
        if size == 0 || size > 1 << 16 {
            None
        } else {
            Some(Self(size))
        }
    }

    /// The number of distinct sequence numbers.
    pub const fn size(self) -> u32 {
        self.0
    }
}

/// A probe stream's cadence and timeout, together with the sequence space they're
/// used in.
///
/// Can only be constructed when `timeout < space × cadence`. That rule is what
/// keeps a sequence number from being reused while its probe is still in flight
/// (ADR-0023). Because it's enforced here, it can never become a runtime bug.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StreamConfig {
    cadence: Time,
    timeout: Time,
    space: SequenceSpace,
}

/// Why a stream configuration was rejected.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum InvalidStreamConfig {
    /// The cadence is zero or negative.
    #[error("cadence must be positive")]
    NonPositiveCadence,
    /// The timeout is zero or negative.
    #[error("timeout must be positive")]
    NonPositiveTimeout,
    /// The timeout is so long that a sequence number could be reused while still in
    /// flight.
    #[error("timeout must be shorter than the sequence space times the cadence")]
    TimeoutOutlivesSequenceSpace,
}

impl StreamConfig {
    /// Validates a configuration.
    pub fn new(
        cadence: Time,
        timeout: Time,
        space: SequenceSpace,
    ) -> Result<Self, InvalidStreamConfig> {
        if cadence.value <= 0.0 {
            return Err(InvalidStreamConfig::NonPositiveCadence);
        }
        if timeout.value <= 0.0 {
            return Err(InvalidStreamConfig::NonPositiveTimeout);
        }
        if timeout >= cadence * f64::from(space.size()) {
            return Err(InvalidStreamConfig::TimeoutOutlivesSequenceSpace);
        }
        Ok(Self {
            cadence,
            timeout,
            space,
        })
    }

    /// How often the stream sends.
    pub fn cadence(&self) -> Time {
        self.cadence
    }

    /// How long a probe may wait for its reply.
    pub fn timeout(&self) -> Time {
        self.timeout
    }

    /// The sequence space the stream's probes are numbered in.
    pub fn space(&self) -> SequenceSpace {
        self.space
    }
}
