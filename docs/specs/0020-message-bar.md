# Spec 0020: One message bar

Status: **agreed with the maintainer, 2026-11-27** (design round 1, question 6: "Keep the bars, but
at most one line, newest on top", with the note "We should see the error number and click through
each one if we want"). Web app.

## Problem

Up to seven kinds of bar can stack above the map at once: an error, a notice, project progress,
"what opening the project added", "reopen your last project", an imagery note, one bar per engine
warning, and the "opened from an older format" row. Each takes a full line. With a few warnings the
map loses a quarter of its height, and the user must dismiss each one.

## Agreed behaviour

All of them go into **one message bar**, one line high, directly under the menu bar.

- The bar shows the **newest** message.
- On its left: the kind (error, warning, note), shown as a word and a shape, and the colour of the
  worst kind still in the list (error red, warning amber, note grey).
- On its right: "**2 of 5**" and **‹ ›** arrows to step through the other messages, newest first; a
  **Dismiss** button for the message showing; and **Dismiss all**.
- A long message is cut to one line with "…"; click it to open it in full under the bar. Click again
  to fold it.
- Progress lines ("Opening the project…") show in the bar while they last and leave no message
  behind.
- When the list is empty, the bar is gone and the map moves up.
- The "Reopen your last project" bar goes to the start card (spec 0018), not into this bar.
- A new design (New, Open, Open project) starts an empty list, as today.

## Acceptance checks

1. Open a design with three engine warnings: one bar shows, saying "1 of 3"; the map loses one line
   only.
2. The arrows step through all three; the count changes "2 of 3", "3 of 3".
3. Dismiss removes the message showing and the count drops; Dismiss all empties the bar and it
   goes.
4. An error and two warnings: the bar is red while the error is in the list.
5. A message longer than the bar: cut with "…", opens in full on click.
6. Open project: "Opening the project…" shows in the bar, then goes without being counted.
7. New starts with no bar.
