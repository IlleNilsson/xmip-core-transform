//! What each standard transform unit is, as data: its name, the stages it
//! runs at and what a step gives it (`node::unit::Declaration`). Compiled
//! with or without the units' code, so `configure` knows every standard
//! unit without linking an XML reader (the owner, 2026-10-22: the standard
//! module is built in).

use node::Stage;
use node::unit::Declaration;
use xcore::settings::{Applies, Fixed, Kind, Presence, Setting, Settings};

/// Convert's name, as a step names it.
pub const CONVERT: &str = "Convert";
/// The structures Convert reads and writes.
pub const STRUCTURES: &[&str] = &["xml", "json", "csv"];

/// Convert: XML to and from JSON, CSV to and from JSON.
pub const CONVERT_UNIT: &Declaration = &Declaration {
    name: CONVERT,
    capability: "transform",
    stages: &Stage::ALL,
    settings: &Settings {
        technology: CONVERT,
        settings: &[
            Setting {
                name: "from",
                kind: Kind::Choice {
                    choices: STRUCTURES,
                },
                presence: Presence::Required,
                meaning: "The structure the content is read in.",
                applies: Applies::Both,
            },
            Setting {
                name: "to",
                kind: Kind::Choice {
                    choices: STRUCTURES,
                },
                presence: Presence::Required,
                meaning: "The structure the new content is written in.",
                applies: Applies::Both,
            },
            Setting {
                name: "arrays",
                kind: Kind::Text,
                presence: Presence::Optional,
                meaning: "The XML elements written as JSON arrays, their names separated by commas.",
                applies: Applies::Both,
            },
            Setting {
                name: "root",
                kind: Kind::Text,
                presence: Presence::Optional,
                meaning: "The XML element the JSON is written inside, where its top object is not one member.",
                applies: Applies::Both,
            },
            Setting {
                name: "separator",
                kind: Kind::Choice {
                    choices: &[",", ";", "tab", "|"],
                },
                presence: Presence::Default(Fixed::Text(",")),
                meaning: "The character between a CSV record's fields.",
                applies: Applies::Both,
            },
            Setting {
                name: "header",
                kind: Kind::Boolean,
                presence: Presence::Default(Fixed::Boolean(true)),
                meaning: "Whether the CSV's first record names its fields.",
                applies: Applies::Both,
            },
            Setting {
                name: "columns",
                kind: Kind::Text,
                presence: Presence::Optional,
                meaning: "The CSV columns written, in order and separated by commas; else the first item's members.",
                applies: Applies::Both,
            },
        ],
    },
};

/// Stylesheet's name, as a step names it.
pub const STYLESHEET: &str = "Stylesheet";
/// The XSLT versions there are.
pub const VERSIONS: &[&str] = &["1.0", "2.0", "3.0", "4.0"];

/// Stylesheet: XML transformed by an XSLT stylesheet, through the engine
/// the step chooses.
pub const STYLESHEET_UNIT: &Declaration = &Declaration {
    name: STYLESHEET,
    capability: "transform",
    stages: &Stage::ALL,
    settings: &Settings {
        technology: STYLESHEET,
        settings: &[
            Setting {
                name: "engine",
                kind: Kind::Text,
                presence: Presence::Required,
                meaning: "The XSLT engine the stylesheet is applied by, one the node was built with.",
                applies: Applies::Both,
            },
            Setting {
                name: "stylesheet",
                kind: Kind::Address,
                presence: Presence::Required,
                meaning: "The stylesheet's file, relative to the configuration.",
                applies: Applies::Both,
            },
            Setting {
                name: "version",
                kind: Kind::Choice { choices: VERSIONS },
                presence: Presence::Optional,
                meaning: "The XSLT version the stylesheet needs; else the version it declares.",
                applies: Applies::Both,
            },
        ],
    },
};

/// Script's name, as a step names it.
pub const SCRIPT: &str = "Script";

/// Script: content transformed by a script, in a runtime the configuration
/// invites.
pub const SCRIPT_UNIT: &Declaration = &Declaration {
    name: SCRIPT,
    capability: "transform",
    stages: &Stage::ALL,
    settings: &Settings {
        technology: SCRIPT,
        settings: &[
            Setting {
                name: "runtime",
                kind: Kind::Text,
                presence: Presence::Required,
                meaning: "The scripting runtime the script runs in: powershell.",
                applies: Applies::Both,
            },
            Setting {
                name: "script",
                kind: Kind::Address,
                presence: Presence::Required,
                meaning: "The script's file, relative to the configuration.",
                applies: Applies::Both,
            },
        ],
    },
};

/// Every standard transform unit's declaration, in the order a person
/// reads them.
pub const DECLARATIONS: [&Declaration; 3] = [CONVERT_UNIT, STYLESHEET_UNIT, SCRIPT_UNIT];

/// Why a step's scripting `runtime` is refused, where it is: the one rule
/// the configuration's check and a starting node both call (the owner,
/// 2026-10-22: *Keeping JavaScript out of it from the begining is one
/// way*). JavaScript is refused by name, and no setting allows it.
#[must_use]
pub fn script_refusal(runtime: &str) -> Option<String> {
    match runtime.to_ascii_lowercase().as_str() {
        "powershell" => None,
        "javascript" | "js" | "node" | "nodejs" | "ecmascript" | "typescript" => Some(format!(
            "{SCRIPT}: JavaScript is not a runtime Xmip hosts, and no setting allows it; \
             write the step in PowerShell or as a compiled unit"
        )),
        "bash" => Some(format!(
            "{SCRIPT}: Bash is decided and not built yet; PowerShell is"
        )),
        other => Some(format!(
            "{SCRIPT}: '{other}' is not a runtime Xmip hosts; PowerShell is"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_declaration_is_sound_and_a_transform_unit() {
        for declaration in DECLARATIONS {
            assert!(
                declaration.problems().is_empty(),
                "{:?}",
                declaration.problems()
            );
            assert_eq!(declaration.capability, "transform");
        }
    }

    #[test]
    fn javascript_is_refused_by_name_and_powershell_is_hosted() {
        assert_eq!(script_refusal("PowerShell"), None);
        for word in ["javascript", "JS", "node", "TypeScript"] {
            let refused = script_refusal(word).expect("refused");
            assert!(
                refused.contains("JavaScript is not a runtime Xmip hosts"),
                "{refused}"
            );
        }
        assert!(
            script_refusal("bash")
                .expect("refused")
                .contains("not built yet")
        );
        assert!(
            script_refusal("python")
                .expect("refused")
                .contains("'python'")
        );
    }
}
