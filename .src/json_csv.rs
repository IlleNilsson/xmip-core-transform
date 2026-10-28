//! JSON written as CSV, a record at a time, the reverse of `csv_json`: an
//! array whose items are flat objects, written in the columns a step names
//! or the first object's members in their order, with a header record
//! where the step says so; or an array of arrays, each a record. A member
//! no column names, and a value a field cannot hold — an object or an
//! array — is refused in words, never dropped.
//!
//! The JSON is read through `serde_json`'s reader one item at a time: what
//! is held is one record.

use std::fmt;
use std::io::{BufReader, Read, Write};

use message::record::Delimited;
use serde::de::{self, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};

use crate::TransformError;
use crate::convert::CHUNK;
use crate::declaration::CONVERT as NAME;

/// What the step says of the CSV written.
pub(crate) struct Columns<'a> {
    pub delimited: Delimited,
    pub header: bool,
    /// The columns, in order; the first object's members where none.
    pub named: Option<&'a [String]>,
}

/// Write the JSON read from `input` to `output` as CSV.
///
/// # Errors
/// The JSON is not an array of flat objects or of arrays, a member names
/// no column, or reading or writing failed.
pub(crate) fn convert(
    columns: &Columns<'_>,
    input: &mut dyn Read,
    output: &mut dyn Write,
) -> Result<(), TransformError> {
    let mut reading = serde_json::Deserializer::from_reader(BufReader::with_capacity(CHUNK, input));
    let mut records = Records {
        columns,
        named: columns.named.map(<[String]>::to_vec),
        output,
        line: Vec::new(),
        number: 0,
    };
    (&mut records)
        .deserialize(&mut reading)
        .and_then(|()| reading.end())
        .map_err(|failed| TransformError::new(format!("{NAME}: {failed}")))?;
    records
        .output
        .flush()
        .map_err(|failed| TransformError::of_io(NAME, &failed))
}

struct Records<'a, 'c> {
    columns: &'a Columns<'c>,
    named: Option<Vec<String>>,
    output: &'a mut dyn Write,
    line: Vec<u8>,
    number: usize,
}

impl Records<'_, '_> {
    fn write<E: de::Error>(&mut self, fields: &[&str]) -> Result<(), E> {
        self.line.clear();
        self.columns
            .delimited
            .write_record(fields.iter().map(|field| field.as_bytes()), &mut self.line);
        self.output
            .write_all(&self.line)
            .map_err(|failed| E::custom(format!("the CSV could not be written: {failed}")))
    }

    fn row<E: de::Error>(&mut self, row: Row) -> Result<(), E> {
        self.number += 1;
        match row {
            Row::Listed(fields) => {
                let fields: Vec<&str> = fields.iter().map(String::as_str).collect();
                self.write(&fields)
            }
            Row::Named(members) => {
                if self.named.is_none() {
                    self.named = Some(members.iter().map(|(name, _)| name.clone()).collect());
                }
                let names = self.named.clone().unwrap_or_default();
                if self.number == 1 && self.columns.header {
                    let header: Vec<&str> = names.iter().map(String::as_str).collect();
                    self.write(&header)?;
                }
                let mut fields = vec![""; names.len()];
                for (name, value) in &members {
                    let at = names
                        .iter()
                        .position(|column| column == name)
                        .ok_or_else(|| {
                            E::custom(format!(
                                "item {} holds '{name}', which no column names",
                                self.number
                            ))
                        })?;
                    fields[at] = value;
                }
                self.write(&fields)
            }
        }
    }
}

impl<'de> DeserializeSeed<'de> for &mut Records<'_, '_> {
    type Value = ();

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_seq(self)
    }
}

impl<'de> Visitor<'de> for &mut Records<'_, '_> {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an array of records")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut items: A) -> Result<(), A::Error> {
        while let Some(row) = items.next_element::<Row>()? {
            self.row::<A::Error>(row)?;
        }
        Ok(())
    }
}

/// One item: a flat object's members in their order, or an array's
/// values.
enum Row {
    Named(Vec<(String, String)>),
    Listed(Vec<String>),
}

impl<'de> de::Deserialize<'de> for Row {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(RowVisitor)
    }
}

struct RowVisitor;

impl<'de> Visitor<'de> for RowVisitor {
    type Value = Row;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a flat object or an array of values")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Row, A::Error> {
        let mut members = Vec::new();
        while let Some(name) = map.next_key::<String>()? {
            let value = map.next_value::<Field>()?;
            members.push((name, value.0));
        }
        Ok(Row::Named(members))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut values: A) -> Result<Row, A::Error> {
        let mut fields = Vec::new();
        while let Some(value) = values.next_element::<Field>()? {
            fields.push(value.0);
        }
        Ok(Row::Listed(fields))
    }
}

/// A value one field holds.
struct Field(String);

impl<'de> de::Deserialize<'de> for Field {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(FieldVisitor)
    }
}

struct FieldVisitor;

impl Visitor<'_> for FieldVisitor {
    type Value = Field;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("text, a number, a boolean or null, which a field holds")
    }

    fn visit_str<E: de::Error>(self, text: &str) -> Result<Field, E> {
        Ok(Field(text.to_string()))
    }

    fn visit_bool<E: de::Error>(self, value: bool) -> Result<Field, E> {
        Ok(Field(value.to_string()))
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Field, E> {
        Ok(Field(value.to_string()))
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Field, E> {
        Ok(Field(value.to_string()))
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Field, E> {
        Ok(Field(value.to_string()))
    }

    fn visit_unit<E: de::Error>(self) -> Result<Field, E> {
        Ok(Field(String::new()))
    }
}
