---
status: accepted
---
# Stages are pure functions with typed, versioned contracts

ADR 0004 made one channel design document, shared by the CLI and the app, that holds every stage's
results. It said nothing about how a stage reads its inputs. Today each stage is a method on the
design session that reads and writes the whole JSON document. A stage finds another stage's result
by looking it up by name in that JSON, some stages rebuild a result instead of reading it, and only
the whole document has a schema. The CLI and the shell behind the web app call the same session
methods, but with different entry points and arguments, and the CLI wraps the bank check in its own
copy. So a stage cannot be tested alone, a stage's output has no contract a reader can
trust, and a new stage (a weir search, a cost, a structures tool) has no seam to plug into.

The maintainer wants OWD modular around standard inputs and outputs, so that it is easy to test and
to grow. Spec 0017 (stage contracts) sets out each stage. This ADR fixes the rules they all follow.

## Decision

1. **A stage is a pure function.** It takes typed inputs and returns one typed output or one typed
   error. It reads no file, no clock, no network and no global state. Its inputs are the run
   settings it needs and the outputs of earlier stages, passed in as values. It never sees the
   whole document.
2. **Every stage input and output is a named Rust type with a JSON schema and a version.** The
   schema is generated from the type, published beside the document schema, and checked in CI. A
   change that alters what an output means raises its version.
3. **A stage reads an earlier result by typed reference.** The runner hands it the value of the type
   it declares. No stage searches the document by name (for example for an option with a given
   method), and no stage rebuilds an earlier stage's result.
4. **The document is a store of stage results, not the stages' working memory.** It holds the run's
   inputs and, for each stage, the output value, its schema version and the hash of what it was made
   from. Staleness works as ADR 0004 says. The runner loads inputs from the store, calls the stage,
   and writes the output back.
5. **One runner, used by the CLI and the web app.** Both call the same stage functions through the
   same runner in `owd-shell`, with the same request type for each stage. A front end gathers inputs
   and shows outputs; it holds no stage logic and no copy of a check.
6. **The stage graph is data.** Each stage declares the types it reads. The runner orders stages
   from those declarations, so an optional stage (weirs and locks) or a later one (cost) adds to the
   graph without changing the stages before it.
7. **Out-of-scope tools get a typed seam, not code.** Structures, hydraulics, sediment and bank
   extraction are stages whose input and output types are defined now. A seam is filled by a given
   input (a file the engineer supplies) until a tool exists to fill it.
8. **One design per run.** A run makes one design line and one set of results. Options are made by
   running OWD several times with different settings, from the CLI or the web app. The document
   does not hold several alternative design lines for a later stage to choose between.
9. **Drawings and reports read stored outputs.** They take the outputs they show as typed values.
   They never re-run a stage or re-walk a line.

## Consequences

- Each stage can be tested alone from a small JSON fixture of its input types, with no session.
- A stage output read by a drawing, a report, the web app or a later tool has a schema it can
  check.
- The document format changes (from 0.4 to 0.5). Older files open, are read into the new types, and
  are saved in the new form; stage results that cannot be read are marked stale and re-run.
- Today's "options" list in the alignment result becomes one design line plus a record of the method
  that made it. A comparison of options moves out of the core.
- Shared value types (terrain sampler, elevation model, river polygon with holes, refusal, water
  level profile) are defined once in `owd-format` and used by every stage.

## Considered options

- **Keep session methods, add schemas for stage results only.** Rejected: a stage would still read
  the whole document and find its inputs by name, so it still could not be tested alone.
- **One crate per stage.** Rejected for now: the stages share many value types, and the crate split
  adds build cost with no gain over modules behind typed functions. It can follow later.
- **Hold several design-line options in one document.** Rejected by the maintainer: one run makes one
  design; options are a layer on top.
