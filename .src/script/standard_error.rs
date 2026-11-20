//! A script runtime's standard error, drained while the runtime runs.
//!
//! A runtime that says more on its standard error than the pipe holds
//! blocks until someone reads it, and a runtime blocked there never answers
//! the Message it was handed. So one thread of its own reads the pipe from
//! the start to its end, keeps the last 64 KiB and counts what it dropped
//! before them: a runtime may say as much as it likes, and what it said
//! last is there, bounded, when it ends (ADR-0066, amendment 2026-11-19).
//!
//! The drain ends when the pipe does, which is when every process holding
//! its writing end has ended: a process the script started inherits it, so
//! the runtime's whole tree is ended before the drain is joined
//! (`process_tree`).

use std::collections::VecDeque;
use std::io::{ErrorKind, Read};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use xcore::lock::Locked;

/// How much of what a runtime said is kept: the last 64 KiB.
const KEPT: usize = 64 * 1024;

/// What one read takes from the pipe at most.
const PIECE: usize = 8 * 1024;

/// A runtime's standard error and the one thread draining it.
pub(super) struct StandardError {
    tail: Arc<Mutex<Tail>>,
    drain: Option<JoinHandle<()>>,
}

impl StandardError {
    /// `said` drained by a thread of its own until it ends.
    ///
    /// # Errors
    /// The thread cannot be started.
    pub(super) fn drained(said: impl Read + Send + 'static) -> std::io::Result<Self> {
        let tail = Arc::new(Mutex::new(Tail::default()));
        let kept = Arc::clone(&tail);
        let drain = std::thread::Builder::new()
            .name("xmip-script-standard-error".to_string())
            .spawn(move || drain(said, &kept))?;
        Ok(Self {
            tail,
            drain: Some(drain),
        })
    }

    /// What the runtime said last, read once its standard error has ended,
    /// so it is asked once the runtime is reaped. The bytes dropped before
    /// the last 64 KiB are counted, in words, in front of them.
    pub(super) fn last_words(&mut self) -> String {
        self.join();
        let mut tail = self.tail.locked();
        let dropped = tail.dropped;
        let kept = tail.kept.make_contiguous();
        // Where bytes were dropped the kept ones may start inside a
        // character: they start at the next one, the rest counted dropped.
        let inside = if dropped == 0 {
            0
        } else {
            kept.iter()
                .take(3)
                .take_while(|byte| **byte & 0xC0 == 0x80)
                .count()
        };
        let said = String::from_utf8_lossy(&kept[inside..]);
        let said = said.trim();
        let dropped = dropped.saturating_add(inside as u64);
        match (dropped, said.is_empty()) {
            (0, true) => "it ended without a word".into(),
            (0, false) => said.to_string(),
            (dropped, _) => format!("({dropped} earlier bytes dropped) {said}"),
        }
    }

    /// What the drain keeps, watched without holding it: once the drain is
    /// joined and this dropped, nothing is left to watch.
    #[cfg(test)]
    pub(super) fn watched(&self) -> std::sync::Weak<Mutex<Tail>> {
        Arc::downgrade(&self.tail)
    }

    fn join(&mut self) {
        if let Some(drain) = self.drain.take() {
            let _ = drain.join();
        }
    }
}

impl Drop for StandardError {
    /// The drain joined once its pipe has ended.
    fn drop(&mut self) {
        self.join();
    }
}

/// The last [`KEPT`] bytes a runtime said, and how many came before them.
#[derive(Default)]
pub(super) struct Tail {
    kept: VecDeque<u8>,
    dropped: u64,
}

impl Tail {
    fn keep(&mut self, piece: &[u8]) {
        self.kept.extend(piece);
        let beyond = self.kept.len().saturating_sub(KEPT);
        if beyond > 0 {
            self.kept.drain(..beyond);
            self.dropped = self
                .dropped
                .saturating_add(u64::try_from(beyond).unwrap_or(u64::MAX));
        }
    }
}

/// `said` read to its end, a piece at a time, into `tail`.
fn drain(mut said: impl Read, tail: &Mutex<Tail>) {
    let mut piece = vec![0u8; PIECE];
    loop {
        match said.read(&mut piece) {
            Ok(0) => return,
            Ok(read) => tail.locked().keep(&piece[..read]),
            Err(failed) if failed.kind() == ErrorKind::Interrupted => {}
            Err(_) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[expect(non_snake_case, reason = "KiB is spelled as the unit is")]
    fn the_last_words_keep_64_KiB_count_the_rest_and_start_on_a_character() {
        // 1 MiB of 'x', then a character of two bytes straddling the cut.
        let mut said = vec![b'x'; 1024 * 1024];
        said.extend("é".as_bytes());
        said.extend(vec![b'y'; KEPT - 1]);
        let mut errors = StandardError::drained(std::io::Cursor::new(said)).expect("drained");
        let words = errors.last_words();
        let dropped = 1024 * 1024 + 2;
        assert_eq!(
            words,
            format!("({dropped} earlier bytes dropped) {}", "y".repeat(KEPT - 1))
        );
    }

    #[test]
    fn a_runtime_that_said_nothing_ended_without_a_word() {
        let mut errors = StandardError::drained(std::io::empty()).expect("drained");
        assert_eq!(errors.last_words(), "it ended without a word");
    }
}
