//! Convert carries content far larger than the pieces it holds in bounded
//! memory, both ways. How fast a small Message is carried is the module's
//! timing program's (`tests/measured`).
//!
//! One test in a program of its own: the allocator's count is the whole
//! process's, so nothing runs beside it.

#[path = "measured/content.rs"]
mod content;

use std::io;
use std::thread;
use std::time::Instant;

use content::{CASES, Made, Sink, given, nothing};
use xmip_core_transform::{Stage, open};

#[global_allocator]
static HELD: xcore::held::Counted = xcore::held::Counted;

/// What Convert may hold however large the content: a few pieces, the
/// names open on the way down and one record.
const BOUND: usize = 4 * 1024 * 1024;

/// `made` converted there by `there` and back by `back`, the two joined by
/// a pipe so neither's output is ever whole.
fn there_and_back(made: Made, there: &[(&str, &str)], back: &[(&str, &str)]) -> Sink {
    let outward = open("Convert", Stage::Receive, &given(there)).expect("opens");
    let inward = open("Convert", Stage::Send, &given(back)).expect("opens");
    let (mut reading, mut writing) = io::pipe().expect("a pipe");
    let going = thread::spawn(move || {
        let mut made = made;
        outward
            .transform(&nothing(), &mut made, &mut writing)
            .expect("there");
    });
    let mut sink = Sink::default();
    inward
        .transform(&nothing(), &mut reading, &mut sink)
        .expect("back");
    going.join().expect("the far end");
    sink
}

#[test]
fn convert_carries_large_content_in_bounded_memory() {
    for case in CASES {
        // About 64 MiB of content.
        let count = 800_000;
        let mut expected = Sink::default();
        io::copy(&mut (case.made)(count), &mut expected).expect("made");
        let started = Instant::now();
        let (held, sink) =
            xcore::held::above(|| there_and_back((case.made)(count), case.there, case.back));
        println!(
            "{}: {} MiB in {:?}, {} KiB held at most",
            case.what,
            expected.length / 1024 / 1024,
            started.elapsed(),
            held / 1024
        );
        assert!(held < BOUND, "{}: held {held} bytes", case.what);
        assert_eq!(sink, expected, "{}: what came back is what went", case.what);
    }
}
