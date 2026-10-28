//! JSON written as XML, a piece at a time, the reverse of `xml_json`: a
//! member is an element named for it, a member named `@` and a name an
//! attribute of the element it is in, `#text` its text, an array the
//! element repeated, `null` an empty element, and every other value its
//! text. Attributes come first, as `xml_json` writes them; one after a
//! child cannot go into a start tag already written, and is refused.
//!
//! The JSON is read through `serde_json`'s reader with visitors that write
//! as they visit, so no document is built: what is held is the element
//! names open on the way down and one string at a time.

use std::fmt;
use std::io::{BufReader, Read, Write};

use serde::de::{self, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};

use crate::TransformError;
use crate::declaration::CONVERT as NAME;

/// Write the JSON read from `input` to `output` as XML: inside one element
/// called `root` where it is named, else as the one member the top object
/// holds.
///
/// # Errors
/// The JSON is not well formed, a name is no XML name, an attribute comes
/// after a child, the top object holds more than one member and no root
/// is named, or reading or writing failed.
pub(crate) fn convert(
    root: Option<&str>,
    input: &mut dyn Read,
    output: &mut dyn Write,
) -> Result<(), TransformError> {
    let mut reading = serde_json::Deserializer::from_reader(BufReader::with_capacity(
        crate::convert::CHUNK,
        input,
    ));
    let mut out = Out { output };
    let written = match root {
        Some(root) => Repeated {
            name: root,
            out: &mut out,
        }
        .deserialize(&mut reading),
        None => Top { out: &mut out }.deserialize(&mut reading),
    };
    written
        .and_then(|()| reading.end())
        .map_err(|failed| TransformError::new(format!("{NAME}: {failed}")))?;
    out.output
        .flush()
        .map_err(|failed| TransformError::of_io(NAME, &failed))
}

/// Where the XML goes.
struct Out<'a> {
    output: &'a mut dyn Write,
}

impl Out<'_> {
    fn put<E: de::Error>(&mut self, text: &str) -> Result<(), E> {
        self.output
            .write_all(text.as_bytes())
            .map_err(|failed| E::custom(format!("the XML could not be written: {failed}")))
    }

    fn text<E: de::Error>(&mut self, text: &str) -> Result<(), E> {
        self.put(&codec::xml::escape(text))
    }
}

/// An XML name, as far as a JSON member's name can be one: a letter or `_`
/// first, then letters, digits, `.`, `-`, `_` and `:`.
fn named<E: de::Error>(name: &str) -> Result<&str, E> {
    let mut characters = name.chars();
    let first = characters
        .next()
        .is_some_and(|first| first.is_alphabetic() || first == '_');
    if first && characters.all(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | ':')) {
        Ok(name)
    } else {
        Err(E::custom(format!("'{name}' is no XML name")))
    }
}

/// The top object, whose one member is the root.
struct Top<'a, 'b> {
    out: &'a mut Out<'b>,
}

impl<'de> DeserializeSeed<'de> for Top<'_, '_> {
    type Value = ();

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_map(self)
    }
}

impl<'de> Visitor<'de> for Top<'_, '_> {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an object of one member, the root element")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        let Some(name) = map.next_key::<String>()? else {
            return Err(de::Error::custom(
                "the top object holds no member to be the root",
            ));
        };
        map.next_value_seed(Element {
            name: &name,
            out: self.out,
        })?;
        if let Some(second) = map.next_key::<String>()? {
            return Err(de::Error::custom(format!(
                "the top object holds '{second}' beside '{name}'; name the root"
            )));
        }
        Ok(())
    }
}

/// One member's value, written as the element `name`.
struct Element<'a, 'b, 'n> {
    name: &'n str,
    out: &'a mut Out<'b>,
}

impl<'de> DeserializeSeed<'de> for Element<'_, '_, '_> {
    type Value = ();

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl Element<'_, '_, '_> {
    fn scalar<E: de::Error>(self, text: &str) -> Result<(), E> {
        let name = named(self.name)?;
        self.out.put(&format!("<{name}>"))?;
        self.out.text(text)?;
        self.out.put(&format!("</{name}>"))
    }
}

impl<'de> Visitor<'de> for Element<'_, '_, '_> {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "the value of '{}'", self.name)
    }

    fn visit_str<E: de::Error>(self, text: &str) -> Result<(), E> {
        self.scalar(text)
    }

    fn visit_bool<E: de::Error>(self, value: bool) -> Result<(), E> {
        self.scalar(if value { "true" } else { "false" })
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<(), E> {
        self.scalar(&value.to_string())
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<(), E> {
        self.scalar(&value.to_string())
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<(), E> {
        self.scalar(&value.to_string())
    }

    fn visit_unit<E: de::Error>(self) -> Result<(), E> {
        let name = named(self.name)?;
        self.out.put(&format!("<{name}/>"))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<(), A::Error> {
        while sequence
            .next_element_seed(Repeated {
                name: self.name,
                out: &mut *self.out,
            })?
            .is_some()
        {}
        Ok(())
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        let name = named::<A::Error>(self.name)?;
        self.out.put::<A::Error>(&format!("<{name}"))?;
        let mut opened = false;
        while let Some(key) = map.next_key::<String>()? {
            if let Some(attribute) = key.strip_prefix('@') {
                if opened {
                    return Err(de::Error::custom(format!(
                        "the attribute '{key}' of '{name}' comes after a child"
                    )));
                }
                let attribute = named::<A::Error>(attribute)?;
                let value = map.next_value::<Scalar>()?;
                self.out.put::<A::Error>(&format!(" {attribute}=\""))?;
                self.out.text::<A::Error>(&value.0)?;
                self.out.put::<A::Error>("\"")?;
                continue;
            }
            if !opened {
                self.out.put::<A::Error>(">")?;
                opened = true;
            }
            if key == "#text" {
                let value = map.next_value::<Scalar>()?;
                self.out.text::<A::Error>(&value.0)?;
            } else {
                map.next_value_seed(Element {
                    name: &key,
                    out: &mut *self.out,
                })?;
            }
        }
        if opened {
            self.out.put(&format!("</{name}>"))
        } else {
            self.out.put("/>")
        }
    }
}

/// One element exactly: an item of an array, the element repeated, or the
/// root a step names. An array there has no element to repeat and is
/// refused.
struct Repeated<'a, 'b, 'n> {
    name: &'n str,
    out: &'a mut Out<'b>,
}

impl<'de> DeserializeSeed<'de> for Repeated<'_, '_, '_> {
    type Value = ();

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Repeated<'_, '_, '_> {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "an item of '{}'", self.name)
    }

    fn visit_str<E: de::Error>(self, text: &str) -> Result<(), E> {
        Element {
            name: self.name,
            out: self.out,
        }
        .visit_str(text)
    }

    fn visit_bool<E: de::Error>(self, value: bool) -> Result<(), E> {
        Element {
            name: self.name,
            out: self.out,
        }
        .visit_bool(value)
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<(), E> {
        Element {
            name: self.name,
            out: self.out,
        }
        .visit_i64(value)
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<(), E> {
        Element {
            name: self.name,
            out: self.out,
        }
        .visit_u64(value)
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<(), E> {
        Element {
            name: self.name,
            out: self.out,
        }
        .visit_f64(value)
    }

    fn visit_unit<E: de::Error>(self) -> Result<(), E> {
        Element {
            name: self.name,
            out: self.out,
        }
        .visit_unit()
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<(), A::Error> {
        Element {
            name: self.name,
            out: self.out,
        }
        .visit_map(map)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, _: A) -> Result<(), A::Error> {
        Err(de::Error::custom(format!(
            "'{}' holds an array where one element belongs",
            self.name
        )))
    }
}

/// A value written as text: an attribute's or `#text`'s.
struct Scalar(String);

impl<'de> de::Deserialize<'de> for Scalar {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(ScalarVisitor)
    }
}

struct ScalarVisitor;

impl Visitor<'_> for ScalarVisitor {
    type Value = Scalar;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("text, a number, a boolean or null")
    }

    fn visit_str<E: de::Error>(self, text: &str) -> Result<Scalar, E> {
        Ok(Scalar(text.to_string()))
    }

    fn visit_bool<E: de::Error>(self, value: bool) -> Result<Scalar, E> {
        Ok(Scalar(value.to_string()))
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Scalar, E> {
        Ok(Scalar(value.to_string()))
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Scalar, E> {
        Ok(Scalar(value.to_string()))
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Scalar, E> {
        Ok(Scalar(value.to_string()))
    }

    fn visit_unit<E: de::Error>(self) -> Result<Scalar, E> {
        Ok(Scalar(String::new()))
    }
}
