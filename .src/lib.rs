#![forbid(unsafe_code)]

//! Transformation: one Message's content into a new Message's (the owner,
//! 2026-10-22: *A transformation is not a prepare*). A transform unit reads
//! the Stream of the section it transforms and writes the new content as
//! it goes; the runtime keeps what it wrote as the new Stream and makes
//! the new Message generation over it (ADR-0013), never editing either.
//!
//! The standard transform units run at receive, at process and at send:
//! `convert` between XML, JSON and CSV, `stylesheet` through an XSLT engine
//! the step chooses, and `script` through a scripting runtime the
//! configuration invites. A designer's map and a user's own code are
//! transform units of their own modules, called the same way.
//!
//! What each unit is, its [`declaration`], is compiled always; the units'
//! code and its dependencies come with the default feature `units`, so the
//! configuration knows the standard units without linking them.

pub mod declaration;

#[cfg(feature = "units")]
pub mod convert;
#[cfg(feature = "units")]
mod csv_json;
#[cfg(feature = "units")]
mod json_csv;
#[cfg(feature = "units")]
mod json_xml;
#[cfg(feature = "units")]
pub mod script;
#[cfg(feature = "units")]
pub mod stylesheet;
#[cfg(feature = "units")]
mod xml_json;

pub use declaration::DECLARATIONS;
pub use node::Stage;

#[cfg(feature = "units")]
pub use units::*;

#[cfg(feature = "units")]
mod units {
    use std::io::{Read, Write};

    use message::Message;
    use xcore::settings::Given;

    use crate::Stage;

    /// What a step gave a unit, read through its declaration.
    pub type Values = xcore::settings::Read;

    xcore::declare_error!(TransformError);

    impl TransformError {
        /// What reading or writing the content refused, named by the unit.
        #[must_use]
        pub fn of_io(unit: &str, failed: &std::io::Error) -> Self {
            Self::new(format!("{unit}: {failed}"))
        }
    }

    /// One transform unit, opened for one step.
    pub trait Transform: Send + Sync {
        /// Write the new content of `message` into `output`, reading the
        /// content it transforms from `input` a piece at a time.
        ///
        /// # Errors
        /// The content is not what the unit reads, or what it says cannot
        /// be written as what the unit writes; or reading or writing failed.
        fn transform(
            &self,
            message: &Message,
            input: &mut dyn Read,
            output: &mut dyn Write,
        ) -> Result<(), TransformError>;
    }

    /// A standard transform unit, linked: its declaration and how it opens.
    pub type TransformUnit = node::unit::Unit<dyn Transform>;

    /// The standard transform units this crate opens by itself, in the order
    /// a person reads them. [`crate::stylesheet`]'s is opened over the
    /// engines the module linking them hands in.
    pub static UNITS: [&TransformUnit; 2] = [&crate::convert::UNIT, &crate::script::UNIT];

    /// The standard transform unit called `name`, of those this crate opens.
    #[must_use]
    pub fn unit(name: &str) -> Option<&'static TransformUnit> {
        UNITS
            .iter()
            .copied()
            .find(|unit| unit.declaration.name == name)
    }

    /// Open the standard transform unit called `name` for a step at `stage`
    /// that gave `given`.
    ///
    /// # Errors
    /// No standard transform unit this crate opens is called `name`, or it
    /// refuses the stage or the settings.
    pub fn open(
        name: &str,
        stage: Stage,
        given: &[(String, Given)],
    ) -> Result<Box<dyn Transform>, TransformError> {
        unit(name)
            .ok_or_else(|| {
                TransformError::new(format!("no standard transform unit is called '{name}'"))
            })?
            .open(stage, given)
            .map_err(TransformError::new)
    }

    /// A Message of nothing, for a unit's tests: the unit reads its input.
    #[cfg(test)]
    pub(crate) fn nothing() -> Message {
        Message::received(
            xcore::MessageId::new(1),
            Vec::new(),
            context::MessageContext::new(),
            message::MessageTreatment::BUSINESS,
        )
    }

    /// `input` transformed by the standard unit called `name` at receive
    /// with `given`, a boolean as a boolean and anything else as text.
    #[cfg(test)]
    pub(crate) fn transformed(
        name: &str,
        given: &[(&str, &str)],
        input: &[u8],
    ) -> Result<Vec<u8>, TransformError> {
        let given: Vec<(String, Given)> = given
            .iter()
            .map(|(name, value)| {
                let value = match *value {
                    "true" => Given::Boolean(true),
                    "false" => Given::Boolean(false),
                    text => Given::Text(text.to_string()),
                };
                ((*name).to_string(), value)
            })
            .collect();
        let mut output = Vec::new();
        open(name, Stage::Receive, &given)?.transform(&nothing(), &mut &input[..], &mut output)?;
        Ok(output)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn every_unit_opens_from_a_declaration_this_crate_gives() {
            for unit in UNITS {
                assert!(crate::DECLARATIONS.contains(&unit.declaration));
                assert!(std::ptr::eq(
                    super::unit(unit.declaration.name).expect("found"),
                    unit
                ));
            }
            let refused = open("Nothing", Stage::Send, &[]).err().expect("refused");
            assert!(refused.to_string().contains("'Nothing'"), "{refused}");
        }
    }
}
