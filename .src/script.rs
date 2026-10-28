//! Script: content transformed by a script the step names, in a runtime
//! the configuration invites (the owner, 2026-10-22: *There are other
//! transformations, like with compled code or scripts like PowerShell, god
//! forbidd JavaScript*; *We have to be lenient to people that does not
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

use std::io::{BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;

use message::Message;

use crate::declaration::{SCRIPT as NAME, SCRIPT_UNIT, script_refusal as refusal};
use crate::{Stage, Transform, TransformError, TransformUnit, Values};

/// Script, as the standard module declares it.
pub const UNIT: TransformUnit = TransformUnit::new(SCRIPT_UNIT, open);

/// The host the runtime runs: it loads the script once, says it is ready,
/// then answers each frame — a length and that many bytes of UTF-8 — with
/// an outcome, a length and the new content, or the failure in words.
const HOST: &str = r"$ErrorActionPreference = 'Stop'
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

/// The runtime's process, warm, with its two ends.
struct Host {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}

impl Host {
    /// The runtime started on `script`, once it said the script loaded.
    fn start(script: &str) -> Result<Self, String> {
        let utf16: Vec<u8> = HOST.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let mut child = Command::new("pwsh")
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-EncodedCommand",
            ])
            .arg(codec::base64::encode(&utf16))
            .env("XMIP_SCRIPT", script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|failed| format!("{NAME}: PowerShell (pwsh) cannot be started: {failed}"))?;
        let (Some(input), Some(output)) = (child.stdin.take(), child.stdout.take()) else {
            return Err(format!(
                "{NAME}: PowerShell's standard streams were not handed over"
            ));
        };
        let mut host = Self {
            child,
            input,
            output: BufReader::new(output),
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

    /// What the runtime said on its error stream before it ended.
    fn failure(&mut self) -> String {
        let _ = self.child.kill();
        let mut said = String::new();
        if let Some(mut errors) = self.child.stderr.take() {
            let _ = errors.read_to_string(&mut said);
        }
        let said = said.trim();
        if said.is_empty() {
            "it ended without a word".into()
        } else {
            said.to_string()
        }
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
            Err(String::from_utf8_lossy(&answer).into_owned())
        })
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
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
        let mut host = self
            .host
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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

    fn script(name: &str, body: &str) -> String {
        let folder =
            std::env::temp_dir().join(format!("xmip-script-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&folder).expect("a folder");
        let place = folder.join("transform.ps1");
        std::fs::write(&place, body).expect("written");
        place.to_string_lossy().into_owned()
    }

    #[test]
    fn javascript_is_refused_by_name_when_the_step_opens() {
        let place = script("javascript", "param($text) $text");
        let refused = transformed(NAME, &[("runtime", "javascript"), ("script", &place)], b"")
            .expect_err("refused");
        assert!(
            refused.to_string().contains("JavaScript is not a runtime"),
            "{refused}"
        );
    }

    #[test]
    fn a_powershell_script_is_loaded_once_and_called_warm_for_each_message() {
        let place = script("upper", "param($text) $text.ToUpperInvariant()");
        let given = [
            (
                "runtime".to_string(),
                xcore::settings::Given::Text("powershell".into()),
            ),
            ("script".to_string(), xcore::settings::Given::Text(place)),
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
        let place = script(
            "throw",
            "param($text) if ($text -eq 'bad') { throw 'no bad orders' }; $text",
        );
        let given = [
            (
                "runtime".to_string(),
                xcore::settings::Given::Text("powershell".into()),
            ),
            ("script".to_string(), xcore::settings::Given::Text(place)),
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
