//! Script: content transformed by a script the step names, in a runtime
//! the configuration invites (the owner, 2026-10-22: *There are other
//! transformations, like with compiled code or scripts like PowerShell, God
//! forbid JavaScript*; *We have to be lenient to people who do not
//! know all languages*). PowerShell is the one built; JavaScript is
//! refused by name, in words, and nothing allows it (`declaration::script_refusal`,
//! the one rule the configuration's check and a starting node both call).
//!
//! The runtime runs beside the node in a process of its own, started when
//! the step is opened and kept warm: it loads the script once and is
//! handed one Message's content at a time over its standard input, never
//! started again per Message. The script is a script block taking the
//! content as text and returning the new content; one call holds the
//! content whole in that process, as a script's text is.
//!
//! A script is never refused for being slow: the operations surfaces
//! report a step's cost per Message and suggest a compiled unit where it
//! is costly (`xmip-core-observe`'s advice).
//!
//! What the runtime says on its standard error is drained while it runs
//! ([`standard_error`]), so it may say as much as it likes without
//! blocking, and its last words are there, bounded, when it ends.

mod standard_error;

use std::io::{BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;
use xcore::lock::Locked;

use message::Message;

use crate::declaration::{SCRIPT as NAME, SCRIPT_UNIT, script_refusal as refusal};
use crate::{Stage, Transform, TransformError, TransformUnit, Values};
use standard_error::StandardError;

/// Script, as the standard module declares it.
pub const UNIT: TransformUnit = TransformUnit::new(SCRIPT_UNIT, open);

/// The program the runtime is: PowerShell.
const POWERSHELL: &str = "pwsh";

/// The host the runtime runs: it loads the script once, says it is ready,
/// then answers each frame — a length and that many bytes of UTF-8 — with
/// an outcome, a length and the new content, or the failure in words. What
/// it says on its standard error is plain text without colors, so its last
/// words read as words: PowerShell started with an encoded command would
/// write them as CLIXML, colored.
const HOST: &str = r"$ErrorActionPreference = 'Stop'; $PSStyle.OutputRendering = 'PlainText'
$reader = [IO.BinaryReader]::new([Console]::OpenStandardInput())
$writer = [IO.BinaryWriter]::new([Console]::OpenStandardOutput())
$transform = [scriptblock]::Create([IO.File]::ReadAllText($env:XMIP_SCRIPT))
$writer.Write([int]0); $writer.Flush()
while ($true) {
    try { $length = $reader.ReadInt32() } catch { break }
    $text = [Text.Encoding]::UTF8.GetString($reader.ReadBytes($length))
    try {
        $said = & $transform $text 3>$null 4>$null 5>$null 6>$null
        $answer = [Text.Encoding]::UTF8.GetBytes([string]$said); $outcome = 0
    } catch {
        $answer = [Text.Encoding]::UTF8.GetBytes($_.ToString()); $outcome = 1
    }
    $writer.Write([int]$outcome); $writer.Write([int]$answer.Length)
    $writer.Write($answer); $writer.Flush()
}";

fn open(_: Stage, values: &Values) -> Result<Box<dyn Transform>, String> {
    if let Some(refused) = refusal(values.text("runtime")) {
        return Err(refused);
    }
    let script = values.text("script").to_string();
    if !std::path::Path::new(&script).is_file() {
        return Err(format!("{NAME}: the script '{script}' is no file"));
    }
    let host = Host::start(&script)?;
    Ok(Box::new(Scripted {
        script,
        host: Mutex::new(Some(host)),
    }))
}

/// The runtime's process, warm, with its two ends and what it says on its
/// standard error. Dropped, the process is killed and reaped, and then the
/// drain joined as `errors` drops, once its pipe has ended.
struct Host {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    errors: StandardError,
}

impl Host {
    /// The runtime started on `script`, once it said the script loaded.
    fn start(script: &str) -> Result<Self, String> {
        let utf16: Vec<u8> = HOST.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let mut child = Command::new(POWERSHELL)
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-OutputFormat",
                "Text",
                "-EncodedCommand",
            ])
            .arg(codec::base64::encode(&utf16))
            .env("XMIP_SCRIPT", script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|failed| format!("{NAME}: PowerShell (pwsh) cannot be started: {failed}"))?;
        let handed = (child.stdin.take(), child.stdout.take(), child.stderr.take());
        let (Some(input), Some(output), Some(errors)) = handed else {
            return Err(abandoned(
                child,
                "PowerShell's standard streams were not handed over",
            ));
        };
        let errors = match StandardError::drained(errors) {
            Ok(drained) => drained,
            Err(failed) => {
                let why = format!("PowerShell's standard error cannot be drained: {failed}");
                return Err(abandoned(child, &why));
            }
        };
        let mut host = Self {
            child,
            input,
            output: BufReader::new(output),
            errors,
        };
        if host.number().ok() != Some(0) {
            let why = host.failure();
            return Err(format!("{NAME}: the script '{script}' did not load: {why}"));
        }
        Ok(host)
    }

    fn number(&mut self) -> std::io::Result<u32> {
        let mut bytes = [0u8; 4];
        self.output.read_exact(&mut bytes)?;
        Ok(u32::from_le_bytes(bytes))
    }

    /// What the runtime said last on its standard error: it is killed and
    /// reaped, and its drain read to the end of the pipe.
    fn failure(&mut self) -> String {
        end(&mut self.child);
        self.errors.last_words()
    }

    /// One content handed over and its answer: the new content, or the
    /// script's failure in words.
    fn call(&mut self, content: &[u8]) -> std::io::Result<Result<Vec<u8>, String>> {
        let length = u32::try_from(content.len())
            .map_err(|_| std::io::Error::other("a script takes content of at most 2 GiB"))?;
        self.input.write_all(&length.to_le_bytes())?;
        self.input.write_all(content)?;
        self.input.flush()?;
        let outcome = self.number()?;
        let length = self.number()?;
        let mut answer = vec![0u8; usize::try_from(length).unwrap_or(0)];
        self.output.read_exact(&mut answer)?;
        Ok(if outcome == 0 {
            Ok(answer)
        } else {
            Err(String::from_utf8_lossy_owned(answer))
        })
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        end(&mut self.child);
    }
}

/// The runtime's process killed and reaped.
fn end(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// `child` ended before it became a [`Host`], and why, in words.
fn abandoned(mut child: Child, why: &str) -> String {
    end(&mut child);
    format!("{NAME}: {why}")
}

/// A step's script, its runtime warm; started again where it ended.
struct Scripted {
    script: String,
    host: Mutex<Option<Host>>,
}

impl Transform for Scripted {
    fn transform(
        &self,
        _: &Message,
        input: &mut dyn Read,
        output: &mut dyn Write,
    ) -> Result<(), TransformError> {
        let mut content = Vec::new();
        input
            .read_to_end(&mut content)
            .map_err(|failed| TransformError::of_io(NAME, &failed))?;
        let mut host = self.host.locked();
        if host.is_none() {
            *host = Some(Host::start(&self.script).map_err(TransformError::new)?);
        }
        let answered = host.as_mut().map(|running| running.call(&content));
        let answer = match answered {
            Some(Ok(answer)) => answer,
            Some(Err(failed)) => {
                let why = host
                    .take()
                    .map_or_else(String::new, |mut ended| ended.failure());
                return Err(TransformError::new(format!(
                    "{NAME}: the runtime of '{}' ended: {failed}; {why}",
                    self.script
                )));
            }
            None => {
                return Err(TransformError::new(format!(
                    "{NAME}: no runtime is running"
                )));
            }
        };
        let written = answer.map_err(|refused| {
            TransformError::new(format!("{NAME}: '{}' failed: {refused}", self.script))
        })?;
        output
            .write_all(&written)
            .and_then(|()| output.flush())
            .map_err(|failed| TransformError::of_io(NAME, &failed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{nothing, transformed};
    use std::time::{Duration, Instant};

    /// Whether PowerShell is there to start. It is an optional tool: where
    /// it is absent, the test that needs it says SKIPPED and fails nothing
    /// (CONTRIBUTING.md, *The toolchain*). Only its start is asked, and the
    /// process is ended at once; any other refusal to start fails the test.
    fn powershell_is_there(test: &str) -> bool {
        let started = Command::new(POWERSHELL)
            .arg("-Version")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        match started {
            Ok(mut child) => {
                end(&mut child);
                true
            }
            Err(absent) if absent.kind() == std::io::ErrorKind::NotFound => {
                eprintln!("SKIPPED: {test}: PowerShell ({POWERSHELL}) is absent");
                false
            }
            Err(failed) => panic!("PowerShell ({POWERSHELL}) cannot be started: {failed}"),
        }
    }

    /// A script written into a folder of its own, which goes with it when
    /// the test ends, passed or failed.
    struct Written {
        folder: std::path::PathBuf,
        path: String,
    }

    impl Drop for Written {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.folder);
        }
    }

    fn script(name: &str, body: &str) -> Written {
        let folder =
            std::env::temp_dir().join(format!("xmip-script-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&folder).expect("a folder");
        let place = folder.join("transform.ps1");
        std::fs::write(&place, body).expect("written");
        Written {
            path: place.to_string_lossy().into_owned(),
            folder,
        }
    }

    /// How long a test waits before it calls its runtime hung: far beyond
    /// any start of PowerShell on a machine shared with other builds, so
    /// only a runtime that never answers reaches it.
    const HUNG: Duration = Duration::from_secs(60);

    /// `work` done on a thread of its own and awaited at most [`HUNG`], so
    /// a runtime that never answers fails the test instead of hanging it.
    fn within<T: Send + 'static>(test: &str, work: impl FnOnce() -> T + Send + 'static) -> T {
        let (told, answer) = std::sync::mpsc::channel();
        let working = std::thread::spawn(move || {
            let _ = told.send(work());
        });
        match answer.recv_timeout(HUNG) {
            Ok(done) => done,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                panic!("{test}: hung past {HUNG:?}")
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                std::panic::resume_unwind(working.join().expect_err("the work panicked"))
            }
        }
    }

    /// The tail of a runtime's diagnostics, its words of dropped bytes
    /// included, is at most this long.
    const SAID: usize = 64 * 1024 + 64;

    #[test]
    #[expect(non_snake_case, reason = "MiB and KiB are spelled as the units are")]
    fn a_script_writing_1_MiB_to_standard_error_completes_and_its_diagnostics_stay_within_64_KiB() {
        const TEST: &str = "a script writing 1 MiB to standard error completes";
        if !powershell_is_there(TEST) {
            return;
        }
        let script = script(
            "loud",
            "param($text) [Console]::Error.Write('x' * 1MB); [Console]::Error.Flush(); $text",
        );
        let path = script.path.clone();
        let said = within(TEST, move || {
            let mut host = Host::start(&path).expect("loads");
            let answer = host.call(b"order a-1").expect("answered");
            assert_eq!(answer.as_deref(), Ok(&b"order a-1"[..]));
            host.failure()
        });
        assert!(said.len() <= SAID, "{} bytes said", said.len());
        assert!(said.contains(" earlier bytes dropped) xxx"), "{said:.200}");
        assert!(said.ends_with("xxx"), "{said:.200}");
    }

    #[test]
    fn non_terminating_errors_beyond_the_pipe_do_not_block() {
        const TEST: &str = "non-terminating errors beyond the pipe do not block";
        if !powershell_is_there(TEST) {
            return;
        }
        // A hundred errors of 4,000 characters each: about 400 KB of words,
        // beyond any pipe, none of them stopping the script.
        let script = script(
            "errors",
            "param($text)
            foreach ($i in 1..100) {
                Write-Error -ErrorAction Continue ('not stopping ' + $i + ' ' + ('y' * 4000))
            }
            $text",
        );
        let path = script.path.clone();
        let said = within(TEST, move || {
            let mut host = Host::start(&path).expect("loads");
            let answer = host.call(b"order a-1").expect("answered");
            assert_eq!(answer.as_deref(), Ok(&b"order a-1"[..]));
            host.failure()
        });
        assert!(said.len() <= SAID, "{} bytes said", said.len());
        assert!(said.contains(" earlier bytes dropped) "), "{said:.200}");
        assert!(said.contains("not stopping 100 "), "{said:.200}");
    }

    #[test]
    fn a_script_failing_to_load_says_its_last_words_bounded() {
        const TEST: &str = "a script failing to load says its last words";
        if !powershell_is_there(TEST) {
            return;
        }
        // Ten unexpected tokens of 20,000 characters each, a letter of
        // their own per line, a to j: the parse error names every one,
        // about 200 KB of words, beyond any pipe. What PowerShell says
        // around them is in its own language; the lines it quotes are the
        // script's, so the last words are known by the line they quote.
        let mut body = String::new();
        for letter in 'a'..='j' {
            body.push_str("1 '");
            body.push_str(&letter.to_string().repeat(20_000));
            body.push_str("'\n");
        }
        let script = script("unloaded", &body);
        let path = script.path.clone();
        let refused = within(TEST, move || Host::start(&path).err().expect("refused"));
        assert!(
            refused.len() <= SAID + script.path.len() + 64,
            "{} bytes",
            refused.len()
        );
        assert!(refused.contains("did not load: ("), "{refused:.200}");
        assert!(
            refused.contains(" earlier bytes dropped) "),
            "{refused:.200}"
        );
        assert!(
            refused.contains(&"j".repeat(1_000)),
            "the last line quoted: {refused:.200}"
        );
        assert!(
            !refused.contains(&"a".repeat(1_000)),
            "the first line dropped: {refused:.200}"
        );
    }

    /// The drain ends only when the runtime's standard error does, which
    /// is when the runtime has ended: nothing left to watch once the host
    /// is dropped is the runtime ended and its drain joined.
    #[test]
    fn dropping_the_host_ends_its_runtime_and_joins_its_drain() {
        const TEST: &str = "dropping the host ends its runtime and joins its drain";
        if !powershell_is_there(TEST) {
            return;
        }
        let script = script(
            "dropped",
            "param($text) [Console]::Error.Write('x' * 1MB); $text",
        );
        let path = script.path.clone();
        let watched = within(TEST, move || {
            let mut host = Host::start(&path).expect("loads");
            let answer = host.call(b"order a-1").expect("answered");
            assert_eq!(answer.as_deref(), Ok(&b"order a-1"[..]));
            let watched = host.errors.watched();
            drop(host);
            watched
        });
        assert!(watched.upgrade().is_none(), "the drain still runs");
    }

    #[test]
    fn javascript_is_refused_by_name_when_the_step_opens() {
        let place = script("javascript", "param($text) $text");
        let given = [("runtime", "javascript"), ("script", place.path.as_str())];
        let refused = transformed(NAME, &given, b"").expect_err("refused");
        assert!(
            refused.to_string().contains("JavaScript is not a runtime"),
            "{refused}"
        );
    }

    #[test]
    fn a_powershell_script_is_loaded_once_and_called_warm_for_each_message() {
        if !powershell_is_there("a PowerShell script is called warm") {
            return;
        }
        let place = script("upper", "param($text) $text.ToUpperInvariant()");
        let given = [
            (
                "runtime".to_string(),
                xcore::settings::Given::Text("powershell".into()),
            ),
            (
                "script".to_string(),
                xcore::settings::Given::Text(place.path.clone()),
            ),
        ];
        let started = Instant::now();
        let opened = UNIT
            .open(Stage::Process, &given)
            .expect("PowerShell starts");
        let start = started.elapsed();
        let mut rounds = Vec::new();
        for _ in 0..20 {
            let mut written = Vec::new();
            let started = Instant::now();
            opened
                .transform(&nothing(), &mut &b"order a-1"[..], &mut written)
                .expect("transformed");
            rounds.push(started.elapsed());
            assert_eq!(written, b"ORDER A-1");
        }
        let spread = xcore::latency::spread("Script, PowerShell, warm", rounds);
        println!("Script started PowerShell and loaded the script in {start:?}");
        // Lenient: a script is never refused for being slow; it is reported,
        // and a warm call is far quicker than a start.
        assert!(
            spread.median < start,
            "{spread:?} against a start of {start:?}"
        );
        assert!(spread.median < Duration::from_secs(1), "{spread:?}");
    }

    #[test]
    fn a_failing_script_says_why_and_the_runtime_stays_warm() {
        if !powershell_is_there("a failing script says why") {
            return;
        }
        let place = script(
            "throw",
            "param($text) if ($text -eq 'bad') { throw 'no bad orders' }; $text",
        );
        let given = [
            (
                "runtime".to_string(),
                xcore::settings::Given::Text("powershell".into()),
            ),
            (
                "script".to_string(),
                xcore::settings::Given::Text(place.path.clone()),
            ),
        ];
        let opened = UNIT.open(Stage::Receive, &given).expect("opens");
        let refused = opened
            .transform(&nothing(), &mut &b"bad"[..], &mut Vec::new())
            .expect_err("refused");
        assert!(refused.to_string().contains("no bad orders"), "{refused}");
        let mut written = Vec::new();
        opened
            .transform(&nothing(), &mut &b"good"[..], &mut written)
            .expect("still warm");
        assert_eq!(written, b"good");
    }
}
