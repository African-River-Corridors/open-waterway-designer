- **Window optimiser lines no longer gain short S-bends at the joins.** A window that made a straight
  between opposite curves shorter than the minimum straight is now undone and keeps the starting
  line there. So a design line made with `--method windows` can keep fewer adopted windows, cost the
  same or more than before, and have fewer short-straight warnings. On a synthetic S-bend (R_min
  300 m, minimum straight 90 m) a window that joined into a 30 m straight is undone (`short_straight`)
  where it was adopted before. Designs with a minimum straight of 0, and windows with no short
  S-bend at their joins, do not move.
