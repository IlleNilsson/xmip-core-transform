//! What Convert is carried with: content made as it is read, a sink that
//! counts it, the settings given, an empty Message and the cases carried.
//! The bounded-memory test (`tests/bounded.rs`) and the module's timing
//! program both drive it; shared, not copied.

use std::io::{self, Read, Write};

use xcore::settings::Given;

/// Content made as it is read: a head, `count` repetitions of a body, a
/// foot; the body's `#` is the repetition's number.
pub struct Made {
    body: &'static str,
    foot: &'static str,
    count: u64,
    at: u64,
    pending: Vec<u8>,
}

impl Made {
    pub fn new(head: &'static str, body: &'static str, foot: &'static str, count: u64) -> Self {
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
pub struct Sink {
    pub length: u64,
    pub sum: u64,
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

pub fn given(settings: &[(&str, &str)]) -> Vec<(String, Given)> {
    settings
        .iter()
        .map(|(name, value)| ((*name).to_string(), Given::Text((*value).to_string())))
        .collect()
}

pub fn nothing() -> message::Message {
    message::Message::received(
        xcore::MessageId::new(1),
        Vec::new(),
        context::MessageContext::new(),
        message::MessageTreatment::BUSINESS,
    )
}

/// A conversion carried there and back.
pub struct Case {
    pub what: &'static str,
    pub made: fn(u64) -> Made,
    pub there: &'static [(&'static str, &'static str)],
    pub back: &'static [(&'static str, &'static str)],
}

const XML_HEAD: &str = "<Orders>";
const XML_BODY: &str =
    "<Order id=\"#\"><Customer>Jane &amp; Co</Customer><Total>12.50</Total></Order>";
const CSV_HEAD: &str = "id,customer,total\r\n";
const CSV_BODY: &str = "#,\"Jane, Co\",12.50\r\n";

pub const CASES: &[Case] = &[
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
