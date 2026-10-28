//! Convert: content in one structure written in another — XML to JSON and
//! back, CSV to JSON and back — a piece at a time, at receive, at process
//! and at send. The two structures are settings, never the unit's name.
//! XML and CSV meet through JSON: a step converting XML to CSV is two
//! steps, because neither says how the other's shape maps onto it.

use std::io::{Read, Write};

use message::Message;
use message::record::Delimited;

use crate::declaration::{CONVERT as NAME, CONVERT_UNIT};
use crate::json_csv::Columns;
use crate::{Stage, Transform, TransformError, TransformUnit, Values};
use crate::{csv_json, json_csv, json_xml, xml_json};

/// How much of the content a conversion reads at a time.
pub(crate) const CHUNK: usize = 64 * 1024;

/// Convert, as the standard module declares it.
pub const UNIT: TransformUnit = TransformUnit::new(CONVERT_UNIT, open);

/// The conversions there are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pair {
    XmlJson,
    JsonXml,
    CsvJson,
    JsonCsv,
}

struct Convert {
    pair: Pair,
    arrays: Vec<String>,
    root: Option<String>,
    delimited: Delimited,
    header: bool,
    columns: Option<Vec<String>>,
}

/// A list a setting gives, its names separated by commas.
fn listed(text: Option<&str>) -> Vec<String> {
    text.map(|text| {
        text.split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .collect()
    })
    .unwrap_or_default()
}

fn open(_: Stage, values: &Values) -> Result<Box<dyn Transform>, String> {
    let pair = match (values.text("from"), values.text("to")) {
        ("xml", "json") => Pair::XmlJson,
        ("json", "xml") => Pair::JsonXml,
        ("csv", "json") => Pair::CsvJson,
        ("json", "csv") => Pair::JsonCsv,
        (from, to) if from == to => {
            return Err(format!("{NAME}: '{from}' to '{to}' converts nothing"));
        }
        (from, to) => {
            return Err(format!(
                "{NAME}: '{from}' is converted to '{to}' through JSON, in two steps"
            ));
        }
    };
    let separator = match values.text("separator") {
        "tab" => b'\t',
        other => other.as_bytes().first().copied().unwrap_or(b','),
    };
    let columns = values
        .optional_text("columns")
        .map(|text| listed(Some(text)));
    Ok(Box::new(Convert {
        pair,
        arrays: listed(values.optional_text("arrays")),
        root: values.optional_text("root").map(str::to_string),
        delimited: Delimited::separated_by(separator),
        header: values.boolean("header"),
        columns,
    }))
}

impl Transform for Convert {
    fn transform(
        &self,
        _: &Message,
        input: &mut dyn Read,
        output: &mut dyn Write,
    ) -> Result<(), TransformError> {
        match self.pair {
            Pair::XmlJson => xml_json::convert(&self.arrays, input, output),
            Pair::JsonXml => json_xml::convert(self.root.as_deref(), input, output),
            Pair::CsvJson => csv_json::convert(self.delimited, self.header, input, output),
            Pair::JsonCsv => {
                let columns = Columns {
                    delimited: self.delimited,
                    header: self.header,
                    named: self.columns.as_deref(),
                };
                json_csv::convert(&columns, input, output)
            }
        }
    }
}

/// `text` as a JSON string, quoted and escaped: the one way the
/// conversions write one.
pub(crate) fn json_string(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| String::from("\"\""))
}

#[cfg(test)]
mod tests {
    use crate::transformed;

    fn text(bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes).into_owned()
    }

    fn convert(from: &str, to: &str, more: &[(&str, &str)], input: &str) -> String {
        let mut given = vec![("from", from), ("to", to)];
        given.extend_from_slice(more);
        text(&transformed(super::NAME, &given, input.as_bytes()).expect("converted"))
    }

    fn refused(from: &str, to: &str, more: &[(&str, &str)], input: &str) -> String {
        let mut given = vec![("from", from), ("to", to)];
        given.extend_from_slice(more);
        transformed(super::NAME, &given, input.as_bytes())
            .expect_err("refused")
            .to_string()
    }

    const ORDER: &str = "<?xml version=\"1.0\"?>\n<Order id=\"A-1\">\n  <Customer>Jane &amp; Co</Customer>\n  \
        <Lines>\n    <Line>one</Line>\n    <Line>two</Line>\n  </Lines>\n  <Note/>\n  \
        <Total currency=\"EUR\">12.50</Total>\n</Order>";

    #[test]
    fn xml_is_read_as_json_with_its_arrays_named() {
        assert_eq!(
            convert("xml", "json", &[("arrays", "Line")], ORDER),
            "{\"Order\":{\"@id\":\"A-1\",\"Customer\":\"Jane & Co\",\"Lines\":{\"Line\":[\"one\",\"two\"]},\
             \"Note\":\"\",\"Total\":{\"@currency\":\"EUR\",\"#text\":\"12.50\"}}}"
        );
    }

    #[test]
    fn json_is_written_as_xml_and_back() {
        let json = convert("xml", "json", &[("arrays", "Line")], ORDER);
        let xml = convert("json", "xml", &[], &json);
        assert_eq!(
            xml,
            "<Order id=\"A-1\"><Customer>Jane &amp; Co</Customer><Lines><Line>one</Line>\
             <Line>two</Line></Lines><Note></Note><Total currency=\"EUR\">12.50</Total></Order>"
        );
        assert_eq!(convert("xml", "json", &[("arrays", "Line")], &xml), json);
    }

    #[test]
    fn json_values_and_a_named_root_are_written_as_xml() {
        assert_eq!(
            convert(
                "json",
                "xml",
                &[("root", "Batch")],
                "{\"n\":1,\"ok\":true,\"none\":null,\"x\":[1,2]}"
            ),
            "<Batch><n>1</n><ok>true</ok><none/><x>1</x><x>2</x></Batch>"
        );
    }

    #[test]
    fn what_xml_or_json_cannot_say_of_the_other_is_refused_in_words() {
        let repeat = refused("xml", "json", &[], "<a><b>1</b><b>2</b></a>");
        assert!(
            repeat.contains("'b' repeats under 'a'; name it in `arrays`"),
            "{repeat}"
        );
        let between = refused(
            "xml",
            "json",
            &[("arrays", "b")],
            "<a><b>1</b><c/><b>2</b></a>",
        );
        assert!(between.contains("after another element"), "{between}");
        let mixed = refused("xml", "json", &[], "<a>text<b/></a>");
        assert!(mixed.contains("holds text beside its children"), "{mixed}");
        let broken = refused("xml", "json", &[], "<a><b></a>");
        assert!(broken.contains("not well formed"), "{broken}");
        let two = refused("json", "xml", &[], "{\"a\":1,\"b\":2}");
        assert!(two.contains("name the root"), "{two}");
        let late = refused("json", "xml", &[], "{\"a\":{\"b\":1,\"@id\":2}}");
        assert!(late.contains("comes after a child"), "{late}");
        let name = refused("json", "xml", &[], "{\"1st\":1}");
        assert!(name.contains("'1st' is no XML name"), "{name}");
    }

    #[test]
    fn csv_is_read_as_json_and_written_back() {
        let csv = "id;note\r\n1;\"two\nlines\"\r\n2;plain\r\n";
        let json = convert("csv", "json", &[("separator", ";")], csv);
        assert_eq!(
            json,
            "[{\"id\":\"1\",\"note\":\"two\\nlines\"},{\"id\":\"2\",\"note\":\"plain\"}]"
        );
        assert_eq!(convert("json", "csv", &[("separator", ";")], &json), csv);
        assert_eq!(
            convert("csv", "json", &[("header", "false")], "a,b\r\nc,d\r\n"),
            "[[\"a\",\"b\"],[\"c\",\"d\"]]"
        );
        assert_eq!(
            convert(
                "json",
                "csv",
                &[("columns", "b,a")],
                "[{\"a\":1,\"b\":true},{\"a\":null}]"
            ),
            "b,a\r\ntrue,1\r\n,\r\n"
        );
    }

    #[test]
    fn what_csv_cannot_hold_is_refused_in_words() {
        let short = refused("csv", "json", &[], "a,b\r\n1\r\n");
        assert!(
            short.contains("record 2 has 1 fields where the header names 2"),
            "{short}"
        );
        let nested = refused("json", "csv", &[], "[{\"a\":{\"b\":1}}]");
        assert!(nested.contains("which a field holds"), "{nested}");
        let stranger = refused("json", "csv", &[("columns", "a")], "[{\"a\":1,\"z\":2}]");
        assert!(
            stranger.contains("'z', which no column names"),
            "{stranger}"
        );
        let through = refused("xml", "csv", &[], "<a/>");
        assert!(through.contains("through JSON"), "{through}");
    }
}
