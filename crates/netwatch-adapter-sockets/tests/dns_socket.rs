//! A DNS socket against the loopback interface, where no resolver listens on the
//! addresses used here: the kernel answers each query with ICMP port unreachable.

#![allow(clippy::unwrap_used)] // Tests may unwrap (ADR-0015).

use std::num::NonZeroU32;
use std::time::Duration;

use netwatch_adapter_sockets::DnsSocket;
use netwatch_probing::{ProbeSequence, ReplyKind, UnreachableCode};

/// The kernel always numbers the loopback interface 1.
const LOOPBACK: NonZeroU32 = NonZeroU32::MIN;

#[tokio::test]
async fn port_unreachable_answers_the_query_it_quotes() {
    // Any 127/8 address is local; nothing serves DNS on this one.
    let socket = DnsSocket::open("127.77.0.1".parse().unwrap(), LOOPBACK).unwrap();
    socket.send(ProbeSequence::new(41)).unwrap();
    socket.send(ProbeSequence::new(42)).unwrap();

    for sequence in [41, 42] {
        let received = tokio::time::timeout(Duration::from_secs(2), socket.recv())
            .await
            .expect("the kernel answers at once")
            .unwrap();
        assert_eq!(received.reply.sequence, ProbeSequence::new(sequence));
        // ICMP port unreachable is type 3, code 3 (ADR-0049).
        assert_eq!(
            received.reply.kind,
            ReplyKind::Unreachable(UnreachableCode(3))
        );
    }
}
