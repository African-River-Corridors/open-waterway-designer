# Spec 0031: Step 7, Drawings and report

Status: **agreed with the maintainer, 2026-11-27** (design round 4, question 2). Web app. Part of
the steps panel (spec 0021). Uses the drawings (spec 0004) and the reports (spec 0014).

## Problem

The drawings are behind a "Sheets" button and the exports behind two more header buttons. Reports
can only be made from the command line. The engineer has no one place to finish a study.

## Agreed behaviour

Step 7 of the steps panel, **Drawings and report**, works on the **preferred option** (spec 0028),
or the active option when none is preferred. Its heading names that option.

When open it shows three groups:

1. **Drawings**: one row per drawing sheet (plan, long section, cross-sections, …), each with a small
   preview, its sheet number and title. Click a row to open that sheet in the Sheets view. "Download
   all sheets" gives them as PDF.
2. **Report**: **Make report**, with a choice of PDF, HTML or Word, and which report (the screening
   report first). It says what it will hold (the preferred option in full, and the options
   comparison table) and, while it runs, its progress. The file downloads when done.
3. **Exports**: GeoJSON (the design line and its checks) and CSV (the quantity table), as today.

Its status: *not run* until the first report or drawing; *out of date* when the option has changed
since the last one made.

A step it needs that is out of date (for example Quantities) is named, with "Run it first"; the
drawings and report still show, marked "from an out-of-date design".

## Acceptance checks

1. Open the example, run all steps, open step 7: each sheet has a row and a preview; the heading
   names the option.
2. Click a sheet: the Sheets view opens at that sheet.
3. Make report as PDF, HTML, Word: each downloads; each says "Screening — not for construction".
4. With two options and B preferred: the report holds B in full and a table comparing A and B.
5. Change B's vessel: step 7 shows *out of date*, and names Quantities to run first.
6. Export GeoJSON and CSV give the same files as today's buttons.
