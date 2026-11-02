//! The module's timing program (`doc/governance/rust-style.md` section
//! 5c): Convert held to the millisecond rule (`CONTRIBUTING.md`: over a
//! millisecond beyond load is a defect). A plain `cargo test` leaves it
//! out; the landing runs it after every other test of the module, alone,
//! one test at a time (`cargo test --test measured --all-features --
//! --test-threads=1`), so what is measured is Xmip's work and not its
//! neighbors':
//!
//! - Convert carries a small Message there and back well within a
//!   millisecond, over the cases `tests/bounded.rs` carries (`content.rs`,
//!   shared).

mod content;

use std::time::{Duration, Instant};

use content::{CASES, Sink, given, nothing};
use xmip_core_transform::{Stage, open};

#[test]
fn convert_carries_a_small_message_at_once() {
    for case in CASES {
        // The small Message measures the units' own work, as a Message pays
        // it: each step opened once, as a node opens it, and no thread or
        // pipe of the harness's in the figure.
        let outward = open("Convert", Stage::Receive, &given(case.there)).expect("opens");
        let inward = open("Convert", Stage::Send, &given(case.back)).expect("opens");
        let message = nothing();
        let mut converted = Vec::new();
        let rounds: Vec<Duration> = (0..50)
            .map(|_| {
                converted.clear();
                let started = Instant::now();
                outward
                    .transform(&message, &mut (case.made)(10), &mut converted)
                    .expect("there");
                inward
                    .transform(&message, &mut &converted[..], &mut Sink::default())
                    .expect("back");
                started.elapsed()
            })
            .collect();
        let spread = xcore::latency::spread(&format!("{}, ten records", case.what), rounds);
        assert!(
            spread.fastest_of_five < Duration::from_millis(1),
            "{}: {spread:?}",
            case.what
        );
    }
}
