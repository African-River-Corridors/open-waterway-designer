# Spec 0022: Step settings windows

Status: **agreed with the maintainer, 2026-11-27** (design round 1, question 4: "a separate
settings window per step"; round 2, question 3). Web app. Goes with spec 0021 (the steps panel).

## Problem

The expert settings of the stages sit open in the left panel. Optimise alone shows the method, the
window length, the cost per metre of line, the cost per m² of land, the corridor limit, two seam
ticks, three straight lengths, the flood cost, the building cost and the limit above bank-full,
each with a Set button, a help line and a "Stored:" line. The engineer cannot see at a glance which
settings differ from the defaults.

## Agreed behaviour

- Each step of the steps panel (spec 0021) that has settings has a **gear** button. It opens that
  step's **settings window**, which floats over the map and can be moved.
- The window lists the step's settings, grouped under short headings, each with its label, its
  value, its unit and one line of help.
- Changes are held in the window until **Apply**. Apply sends them all at once; the steps they
  affect go *out of date*. **Cancel** (or Escape) throws them away.
- A setting not at its default shows a **dot** beside its label. The step's gear shows a dot too
  when any of its settings is not at its default.
- **Reset to defaults** at the bottom sets every setting of the step back to its default (still
  held until Apply).
- A value that is refused (for example a negative cost) is marked on its field with the reason,
  and Apply is greyed until it is fixed.
- One settings window open at a time.

### What goes in which window

| Step | Settings |
|---|---|
| 1 Project and data | Water level and fall, river mouth, downstream point, terrain memory limit |
| The active option (spec 0028) | The run's objective, limits and target level: a gear beside the option picker |
| 2 Vessel and channel | Rule Sets and their inputs, vessel, draught, custom vessel fields |
| 3 Design line | Rolling circle: keep to the water. River-following: seed tolerance |
| 4 Weirs and pools | Method; window length; cost per metre of line; cost per m² of land; keep inside the corridor; seams on straights; free the line at seams and weirs; seam search window; straight between opposite curves; straight through a weir site; cost per hectare flooded; cost per building flooded; most a pool may stand above bank-full |
| 5 Quantities | (none yet: no gear) |

The forcing of each river-following cut-through stays with its row in the results panel
(spec 0023, Cut-throughs tab), not in the window.

## Acceptance checks

1. Open the example, step 4, press the gear: the window lists every optimise setting in the table,
   with today's values.
2. Change the cost per metre of line and press Cancel: nothing changes; no step goes out of date.
3. Change it and press Apply: the value is stored; step 4 and step 5 show *out of date*; the setting
   and the gear show a dot.
4. Reset to defaults, Apply: all dots go; the design holds no non-default optimise setting.
5. Type -1 in a cost: the field shows the reason and Apply is greyed.
6. Escape closes the window without applying.
