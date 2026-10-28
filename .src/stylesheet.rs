//! Stylesheet: XML transformed by an XSLT stylesheet, through the engine
//! the step chooses (the owner, 2026-10-22: *There are many XSLT transform
//! engines, versions, let the end user choose*). The stylesheet is the
//! step's setting; the engine is a technology behind [`StylesheetEngine`],
//! declaring the XSLT versions it implements, so a step asking for a
//! version its engine does not implement is refused in words when the node
//! starts, never at a Message.
//!
//! An engine compiles a stylesheet once, when its step is opened; one that
//! compiles ahead of time does it at design time (ADR-0066). No engine
//! streams the document: XSLT reads a tree, and what one holds is the
//! document it transforms.
//!
//! The engines are handed in by the module that links them, so this crate
//! names none: [`open`] takes them, and the standard module's unit is this
//! declaration over the engines it was built with.

use std::io::{Read, Write};

use message::Message;

pub use crate::declaration::{STYLESHEET as NAME, STYLESHEET_UNIT as DECLARATION};
use crate::{Stage, Transform, TransformError, Values};

/// An XSLT engine: a technology of this capability.
pub trait StylesheetEngine: Send + Sync {
    /// Its name, as a step's `engine` names it.
    fn name(&self) -> &'static str;

    /// The XSLT versions it implements.
    fn versions(&self) -> &'static [&'static str];

    /// `stylesheet`, read from `place`, compiled once. What it includes or
    /// imports is read from beside it and never over a network
    /// (ADR-0045): an engine that can fetch is not admitted.
    ///
    /// # Errors
    /// The stylesheet is not one the engine can compile, in words.
    fn compile(
        &self,
        stylesheet: &str,
        place: &std::path::Path,
    ) -> Result<Box<dyn CompiledStylesheet>, String>;
}

/// A stylesheet compiled by its engine.
pub trait CompiledStylesheet: Send + Sync {
    /// The document read from `input` transformed, written to `output`.
    ///
    /// # Errors
    /// The document is not XML, or the stylesheet fails on it, in words.
    fn apply(&self, input: &mut dyn Read, output: &mut dyn Write) -> Result<(), String>;
}

/// Open the unit for a step at `stage` with `values`, through one of
/// `engines`.
///
/// # Errors
/// No engine is called as the step names, it does not implement the
/// version the step or the stylesheet asks for, the stylesheet cannot be
/// read, or the engine cannot compile it.
pub fn open(
    engines: &[&dyn StylesheetEngine],
    _: Stage,
    values: &Values,
) -> Result<Box<dyn Transform>, String> {
    let named = values.text("engine");
    let engine = engines
        .iter()
        .find(|engine| engine.name() == named)
        .ok_or_else(|| {
            let linked: Vec<&str> = engines.iter().map(|engine| engine.name()).collect();
            format!(
                "{NAME}: no XSLT engine called '{named}' is in this node; it has {}",
                if linked.is_empty() {
                    "none".to_string()
                } else {
                    linked.join(", ")
                }
            )
        })?;
    let place = values.text("stylesheet");
    let stylesheet = std::fs::read_to_string(place)
        .map_err(|failed| format!("{NAME}: the stylesheet '{place}' cannot be read: {failed}"))?;
    let version = match values.optional_text("version") {
        Some(version) => version.to_string(),
        None => declared_version(&stylesheet).ok_or_else(|| {
            format!("{NAME}: the stylesheet '{place}' declares no version, and the step names none")
        })?,
    };
    if !engine.versions().contains(&version.as_str()) {
        return Err(format!(
            "{NAME}: the engine '{named}' implements XSLT {}, and '{place}' needs {version}",
            engine.versions().join(" and ")
        ));
    }
    let compiled = engine
        .compile(&stylesheet, std::path::Path::new(place))
        .map_err(|refused| format!("{NAME}: '{place}' does not compile: {refused}"))?;
    Ok(Box::new(Stylesheet { compiled }))
}

/// The `version` the stylesheet's root element declares, as written.
fn declared_version(stylesheet: &str) -> Option<String> {
    let mut reader = quick_xml::reader::Reader::from_str(stylesheet);
    loop {
        match reader.read_event().ok()? {
            quick_xml::events::Event::Start(root) | quick_xml::events::Event::Empty(root) => {
                let version = root.try_get_attribute("version").ok()??;
                return Some(version.value.trim().to_string());
            }
            quick_xml::events::Event::Eof => return None,
            _ => {}
        }
    }
}

struct Stylesheet {
    compiled: Box<dyn CompiledStylesheet>,
}

impl Transform for Stylesheet {
    fn transform(
        &self,
        _: &Message,
        input: &mut dyn Read,
        output: &mut dyn Write,
    ) -> Result<(), TransformError> {
        self.compiled
            .apply(input, output)
            .map_err(|refused| TransformError::new(format!("{NAME}: {refused}")))
    }
}
