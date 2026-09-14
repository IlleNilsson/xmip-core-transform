# xmip-core-transform

Transformation between declared structured Contracts: a `Transformer` takes a
`TransformRequest` naming the source and target Contracts and produces new
content, and a `TransformRegistry` finds the transformer for a pair. Each
engine is a technology under this repository.

Transformation creates a new Stream and a new Message generation; it never
edits either. It may happen in a Receive Port, an Xmip Process or a Send Port,
while assignment may not. It does not decide acceptance — that is the
Contract's — and it does not move bytes.

`doc/architecture/runtime-model.md` sections 3, 10 and 22 govern where and
what it changes, and ADR-0042 what a Contract holds; `architecture.toml`
carries the maturity.
