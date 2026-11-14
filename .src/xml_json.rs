//! XML read as JSON, a piece at a time: an element is a member named for
//! it, its attributes members of its own named `@` and the attribute's
//! name, its text a string where it holds nothing else, or `#text` beside
//! its attributes; an element with neither text nor anything inside it is
//! `""`. Every value is text, as XML's are.
//!
//! A reader that never looks ahead cannot know an element repeats until it
//! has written the first, so the elements written as JSON arrays are named
//! (`arrays`), as a schema would say; an element repeating under one parent
//! that is not named, or a named one interrupted by another, is refused in
//! words, never written as two members of one name. Text between an
//! element's children is refused for the same reason: JSON's object has
//! no place for it.
//!
//! The XML is read by the estate's one XML reader streamed
//! (`codec::xml::streamed`), which holds a piece at a time, checks that
//! every end tag closes the element it ends and that one root stands alone,
//! and never builds a document.

use std::borrow::Cow;
use std::io::{BufRead, Write};

use codec::xml::streamed::{Streamed, Unread};
use codec::xml::{Event, Tag};

use crate::TransformError;
use crate::convert::json_string;
use crate::declaration::CONVERT as NAME;

/// How far an element has said what it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Form {
    /// Nothing inside it yet but blank text.
    Undecided,
    /// An object: it has attributes or children.
    Object,
    /// A string: it holds text alone.
    Text,
}

/// One element open on the way down.
struct Open {
    name: String,
    form: Form,
    members: usize,
    /// The children's names written, so a repeat is refused.
    children: Vec<String>,
    /// A child named in `arrays` whose array is open.
    array: Option<String>,
    /// The `#text` member, or the element's own string, is open.
    in_text: bool,
    /// Text written to this element.
    texted: bool,
    /// Blank text held until the element says whether it is text.
    blank: String,
}

impl Open {
    fn new(name: String) -> Self {
        Self {
            name,
            form: Form::Undecided,
            members: 0,
            children: Vec::new(),
            array: None,
            in_text: false,
            texted: false,
            blank: String::new(),
        }
    }
}

fn refused(why: impl std::fmt::Display) -> TransformError {
    TransformError::new(format!("{NAME}: {why}"))
}

fn unread(failed: &Unread) -> TransformError {
    match failed {
        Unread::Input(failed) => TransformError::of_io(NAME, failed),
        Unread::Malformed(malformed) => refused(format!(
            "the XML is not well formed near byte {}: {}",
            malformed.at, malformed.reason
        )),
    }
}

/// Write the XML read from `input` to `output` as JSON, writing the
/// elements `arrays` names as arrays.
///
/// # Errors
/// The XML is not well formed, an element repeats unnamed, an element
/// holds text beside its children, or reading or writing failed.
pub(crate) fn convert(
    arrays: &[String],
    input: &mut dyn BufRead,
    output: &mut dyn Write,
) -> Result<(), TransformError> {
    let mut reader = Streamed::document(input).map_err(|failed| unread(&failed))?;
    let mut writer = Writer {
        output,
        arrays,
        open: Vec::new(),
        rooted: false,
    };
    while let Some(piece) = reader.read().map_err(|failed| unread(&failed))? {
        match piece.event {
            Event::Start(tag) => {
                writer.start(tag.name().to_string(), &attributes(&tag)?)?;
                if tag.empty() {
                    writer.end()?;
                }
            }
            Event::End(_) => writer.end()?,
            Event::Text(text) => writer.text(&codec::xml::line_ends(text))?,
            Event::CData(data) => writer.text(&codec::xml::line_ends(data))?,
            Event::Reference(name) => {
                let character = codec::xml::entity(name).map_err(|why| refused(why.message))?;
                writer.text(character.encode_utf8(&mut [0; 4]))?;
            }
            Event::Comment(_) | Event::Instruction { .. } | Event::Doctype(_) => {}
        }
    }
    if !writer.rooted {
        return Err(refused("the XML holds no element"));
    }
    if let Some(open) = writer.open.last() {
        return Err(refused(format!(
            "the element '{}' is never closed",
            open.name
        )));
    }
    writer
        .output
        .flush()
        .map_err(|failed| TransformError::of_io(NAME, &failed))
}

/// A start tag's attributes, each value read as XML reads it.
fn attributes(tag: &Tag<'_>) -> Result<Vec<(String, String)>, TransformError> {
    tag.attributes()
        .map(|written| {
            let written = written.map_err(|failed| refused(format!("an attribute: {failed}")))?;
            let value = codec::xml::attribute_value(written.value)
                .map_err(|failed| refused(format!("an attribute's value: {failed}")))?;
            Ok((written.name.to_string(), value.into_owned()))
        })
        .collect()
}
/// The JSON written so far, and the elements open.
struct Writer<'a> {
    output: &'a mut dyn Write,
    arrays: &'a [String],
    open: Vec<Open>,
    rooted: bool,
}

impl Writer<'_> {
    fn put(&mut self, text: &str) -> Result<(), TransformError> {
        self.output
            .write_all(text.as_bytes())
            .map_err(|failed| TransformError::of_io(NAME, &failed))
    }

    /// A member's name, after a comma where one went before.
    fn member(&mut self, name: &str) -> Result<(), TransformError> {
        let comma = self.open.last().is_some_and(|open| open.members > 0);
        if let Some(open) = self.open.last_mut() {
            open.members += 1;
        }
        let written = format!("{}{}:", if comma { "," } else { "" }, json_string(name));
        self.put(&written)
    }

    /// The parent made an object, ready for a child or an attribute.
    fn as_object(&mut self) -> Result<(), TransformError> {
        self.close_text()?;
        let Some(open) = self.open.last_mut() else {
            return Ok(());
        };
        match open.form {
            Form::Object => Ok(()),
            Form::Text => Err(refused(format!(
                "the element '{}' holds text beside its children",
                open.name
            ))),
            Form::Undecided => {
                open.form = Form::Object;
                self.put("{")
            }
        }
    }

    fn start(
        &mut self,
        name: String,
        attributes: &[(String, String)],
    ) -> Result<(), TransformError> {
        if self.open.is_empty() {
            if self.rooted {
                return Err(refused(format!("a second root element, '{name}'")));
            }
            self.rooted = true;
            let written = format!("{{{}:", json_string(&name));
            self.put(&written)?;
        } else {
            self.child(&name)?;
        }
        self.open.push(Open::new(name));
        if !attributes.is_empty() {
            self.as_object()?;
            for (key, value) in attributes {
                self.member(&format!("@{key}"))?;
                self.put(&json_string(value))?;
            }
        }
        Ok(())
    }

    /// A child called `name` begins in the element open last.
    fn child(&mut self, name: &str) -> Result<(), TransformError> {
        self.as_object()?;
        let repeated = self.arrays.iter().any(|array| array == name);
        let open = self
            .open
            .last_mut()
            .ok_or_else(|| refused("no element is open"))?;
        if open.array.as_deref() == Some(name) {
            return self.put(",");
        }
        let closing = open.array.take().is_some();
        if open.children.iter().any(|child| child == name) {
            let parent = open.name.clone();
            return Err(refused(if repeated {
                format!("'{name}' repeats under '{parent}' after another element")
            } else {
                format!("'{name}' repeats under '{parent}'; name it in `arrays`")
            }));
        }
        open.children.push(name.to_string());
        if repeated {
            open.array = Some(name.to_string());
        }
        if closing {
            self.put("]")?;
        }
        self.member(name)?;
        if repeated { self.put("[") } else { Ok(()) }
    }

    fn text(&mut self, text: &str) -> Result<(), TransformError> {
        let Some(open) = self.open.last_mut() else {
            return if text.trim().is_empty() {
                Ok(())
            } else {
                Err(refused("text outside the root element"))
            };
        };
        if open.in_text {
            return self.put(&escaped(text));
        }
        let blank = text.trim().is_empty();
        match open.form {
            Form::Undecided if blank => {
                open.blank.push_str(text);
                Ok(())
            }
            Form::Undecided => {
                open.form = Form::Text;
                open.in_text = true;
                let held = std::mem::take(&mut open.blank);
                self.put(&format!("\"{}{}", escaped(&held), escaped(text)))
            }
            Form::Object if blank => Ok(()),
            Form::Object if open.texted => Err(refused(format!(
                "the element '{}' holds text beside its children",
                open.name
            ))),
            Form::Object => {
                open.texted = true;
                self.member("#text")?;
                if let Some(open) = self.open.last_mut() {
                    open.in_text = true;
                }
                self.put(&format!("\"{}", escaped(text)))
            }
            Form::Text => self.put(&escaped(text)),
        }
    }

    fn close_text(&mut self) -> Result<(), TransformError> {
        match self.open.last_mut() {
            Some(open) if open.in_text => {
                open.in_text = false;
                self.put("\"")
            }
            _ => Ok(()),
        }
    }

    fn end(&mut self) -> Result<(), TransformError> {
        self.close_text()?;
        let open = self
            .open
            .pop()
            .ok_or_else(|| refused("an end tag closes nothing"))?;
        match open.form {
            Form::Undecided => self.put(&json_string(&open.blank))?,
            Form::Object if open.array.is_some() => self.put("]}")?,
            Form::Object => self.put("}")?,
            Form::Text => {}
        }
        if self.open.is_empty() {
            self.put("}")
        } else {
            Ok(())
        }
    }
}

/// `text` as it is written inside a JSON string, its quotes not included.
fn escaped(text: &str) -> Cow<'_, str> {
    let quoted = json_string(text);
    Cow::Owned(quoted[1..quoted.len() - 1].to_string())
}
