# xmip-core-transform

Transformation: one Message's content into a new Message's (the owner,
2026-10-22: *A transformation is not a prepare, just so we are clear*). A
transform unit reads the Stream of the section it transforms a piece at a
time and writes the new content as it goes; the runtime keeps what it wrote
as the new Stream and makes the new Message generation over it, editing
neither (ADR-0013). It may run at receive, at process and at send, while
assignment is done by a Work Process alone; it does not decide acceptance —
that is the Contract's — and does not move bytes.

The standard transform units, each declared once beside its code as an
`xmip-core-node` `unit::Unit`:

| Unit | Does | Settings |
| --- | --- | --- |
| `Convert` | XML to and from JSON, CSV to and from JSON, streamed | `from`, `to`; `arrays`; `root`; `separator`; `header`; `columns` |
| `Stylesheet` | applies an XSLT stylesheet through the engine the step chooses | `engine`; `stylesheet`; `version` |
| `Script` | runs a script, in PowerShell, kept warm | `runtime`; `script` |

**Convert** never holds the content whole. XML is read by `quick-xml`'s
event reader; JSON by `serde_json`'s reader through visitors that write as
they visit; CSV by the Foundation's walk of delimited records
(`message::record`), a piece at a time. An element is a member named for
it, an attribute a member named `@` and its name, text beside attributes
`#text`. A reader that never looks ahead cannot know an element repeats
until it has written the first, so the elements written as JSON arrays are
named in `arrays`; an element repeating unnamed is refused in words, never
written as two members of one name. XML and CSV meet through JSON, in two
steps.

**Stylesheet** chooses its engine (the owner, 2026-10-22: *There are many
XSLT transform engines, versions, let the end user choose*): an engine is a
technology of this capability behind `stylesheet::StylesheetEngine`,
mounted beside `.src`, declaring the XSLT versions it implements, so a
stylesheet needing a version its engine does not implement is refused when
its step is opened. No engine is built yet (open problem 53). A stylesheet is compiled
when its step is opened, never per Message; what it includes, imports or
reads is answered from beside it and never over a network. XSLT reads a
tree, so a Stylesheet holds the document it transforms.

**Script** runs PowerShell in a process of its own, started when its step
is opened and kept warm: the script is loaded once and handed one Message's
content at a time (the owner, 2026-10-22: *We have to be lenient to people
that does not know all languages*). JavaScript is refused by name, and no
setting allows it (`script::refusal`, the one rule the configuration's
check and a starting node call). A script is never refused for being slow;
the operations surfaces report its cost and suggest a compiled unit
(`xmip-core-observe`'s `advice`).

A designer's map and a user's own compiled code are transform units of
their own modules, called the same way (ADR-0066 clause 4).

`doc/architecture/runtime-model.md` sections 3, 10 and 22 govern where and
what it changes, ADR-0042 what a Contract holds and ADR-0066 the units;
`architecture.toml` carries the maturity.
