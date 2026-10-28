//! CSV read as JSON, a record at a time: an array whose items are objects
//! named by the header record's fields, or arrays of the fields where the
//! CSV has no header. Every value is text, as CSV's are. The records are
//! the Foundation's walk (`message::record`), read a piece at a time, so a
//! CSV is never held whole; a record with more or fewer fields than its
//! header names is refused in words, as the CSV contract refuses it.

use std::io::{Read, Write};

use message::record::{Delimited, Record, Walked};

use crate::TransformError;
use crate::convert::{CHUNK, json_string};
use crate::declaration::CONVERT as NAME;

fn refused(why: impl std::fmt::Display) -> TransformError {
    TransformError::new(format!("{NAME}: {why}"))
}

/// Write the CSV read from `input`, cut as `delimited` says, to `output`
/// as JSON; its first record names the fields where `header` says so.
///
/// # Errors
/// A record cannot be cut, a field is not UTF-8, a record's fields do not
/// match its header, or reading or writing failed.
pub(crate) fn convert(
    delimited: Delimited,
    header: bool,
    input: &mut dyn Read,
    output: &mut dyn Write,
) -> Result<(), TransformError> {
    let mut names: Option<Vec<String>> = None;
    let mut written = 0usize;
    let mut line = String::new();
    put(output, "[")?;
    let walked = delimited.read_records(input, CHUNK, |bytes, record| {
        if record.range.is_empty() {
            return Ok(());
        }
        let fields = fields(delimited, bytes, record)?;
        if header && names.is_none() {
            names = Some(fields);
            return Ok(());
        }
        line.clear();
        if written > 0 {
            line.push(',');
        }
        match &names {
            Some(names) => named(&mut line, names, &fields, record.number)?,
            None => listed(&mut line, &fields),
        }
        written += 1;
        put(output, &line)
    });
    match walked {
        Ok(()) => {}
        Err(Walked::Refused(refusal)) => return Err(refusal),
        Err(Walked::Read(failed)) => return Err(TransformError::of_io(NAME, &failed)),
        Err(Walked::Stopped(why, at)) => {
            return Err(refused(format!(
                "the CSV cannot be read: {why} at byte {at}"
            )));
        }
    }
    put(output, "]")?;
    output
        .flush()
        .map_err(|failed| TransformError::of_io(NAME, &failed))
}

fn put(output: &mut dyn Write, text: &str) -> Result<(), TransformError> {
    output
        .write_all(text.as_bytes())
        .map_err(|failed| TransformError::of_io(NAME, &failed))
}

/// A record's fields as text, unquoted.
fn fields(
    delimited: Delimited,
    bytes: &[u8],
    record: &Record,
) -> Result<Vec<String>, TransformError> {
    record
        .fields
        .iter()
        .map(|range| {
            String::from_utf8(delimited.unquote(&bytes[range.clone()])).map_err(|_| {
                refused(format!(
                    "record {} holds a field that is not UTF-8",
                    record.number
                ))
            })
        })
        .collect()
}

fn named(
    line: &mut String,
    names: &[String],
    fields: &[String],
    number: usize,
) -> Result<(), TransformError> {
    if fields.len() != names.len() {
        return Err(refused(format!(
            "record {number} has {} fields where the header names {}",
            fields.len(),
            names.len()
        )));
    }
    line.push('{');
    for (index, (name, field)) in names.iter().zip(fields).enumerate() {
        if index > 0 {
            line.push(',');
        }
        line.push_str(&json_string(name));
        line.push(':');
        line.push_str(&json_string(field));
    }
    line.push('}');
    Ok(())
}

fn listed(line: &mut String, fields: &[String]) {
    line.push('[');
    for (index, field) in fields.iter().enumerate() {
        if index > 0 {
            line.push(',');
        }
        line.push_str(&json_string(field));
    }
    line.push(']');
}
