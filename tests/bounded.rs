//! Convert carries content far larger than the pieces it holds in bounded
//! memory, both ways, and a small Message well within the millisecond rule
//! (CONTRIBUTING.md: over a millisecond beyond load is a defect).
//!
//! One test in a program of its own: the allocator's count is the whole
//! process's, so nothing runs beside it.

use std::io::{self, Read, Write};
use std::thread;
use std::time::{Duration, Instant};

use xcore::settings::Given;
use xmip_core_transform::{Stage, open};

#[global_allocator]
static HELD: xcore::held::Counted = xcore::held::Counted;

/// What Convert may hold however large the content: a few pieces, the
/// names open on the way down and one record.
const BOUND: usize = 4 * 1024 * 1024;

/// Content made as it is read: a head, `count` repetitions of a body, a
/// foot; the body's `#` is the repetition's number.
struct Made {
    body: &'static str,
    foot: &'static str,
    count: u64,
    at: u64,
    pending: Vec<u8>,
}

impl Made {
    fn new(head: &'static str, body: &'static str, foot: &'static str, count: u64) -> Self {
        Self {
            body,
            foot,
            count,
            at: 0,
            pending: head.as_bytes().to_vec(),
        }
    }
}

impl Read for Made {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        while self.pending.is_empty() && self.at <= self.count {
            self.pending = if self.at < self.count {
                self.body.replace('#', &self.at.to_string()).into_bytes()
            } else {
                self.foot.as_bytes().to_vec()
            };
            self.at += 1;
        }
        let taken = out.len().min(self.pending.len());
        out[..taken].copy_from_slice(&self.pending[..taken]);
        self.pending.drain(..taken);
        Ok(taken)
    }
}

/// What was written, counted and summed, never kept.
#[derive(Default, Debug, PartialEq, Eq)]
struct Sink {
    length: u64,
    sum: u64,
}

impl Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.length += bytes.len() as u64;
        for &byte in bytes {
            self.sum = self.sum.wrapping_mul(31).wrapping_add(u64::from(byte));
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn given(settings: &[(&str, &str)]) -> Vec<(String, Given)> {
    settings
        .iter()
        .map(|(name, value)| ((*name).to_string(), Given::Text((*value).to_string())))
        .collect()
}

fn nothing() -> message::Message {
    message::Message::received(
        xcore::MessageId::new(1),
        Vec::new(),
        context::MessageContext::new(),
        message::MessageTreatment::BUSINESS,
    )
}

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

struct Case {
    what: &'static str,
    made: fn(u64) -> Made,
    there: &'static [(&'static str, &'static str)],
    back: &'static [(&'static str, &'static str)],
}

const XML_HEAD: &str = "<Orders>";
const XML_BODY: &str =
    "<Order id=\"#\"><Customer>Jane &amp; Co</Customer><Total>12.50</Total></Order>";
const CSV_HEAD: &str = "id,customer,total\r\n";
const CSV_BODY: &str = "#,\"Jane, Co\",12.50\r\n";

const CASES: &[Case] = &[
    Case {
        what: "XML to JSON and back",
        made: |count| Made::new(XML_HEAD, XML_BODY, "</Orders>", count),
        there: &[("from", "xml"), ("to", "json"), ("arrays", "Order")],
        back: &[("from", "json"), ("to", "xml")],
    },
    Case {
        what: "CSV to JSON and back",
        made: |count| Made::new(CSV_HEAD, CSV_BODY, "", count),
        there: &[("from", "csv"), ("to", "json")],
        back: &[("from", "json"), ("to", "csv")],
    },
];

#[test]
fn convert_carries_large_content_in_bounded_memory_and_a_small_message_at_once() {
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
