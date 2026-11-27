- **The window optimiser refuses a short straight between opposite curves.** A window whose join
  leaves a straight shorter than the minimum straight (one vessel length by default) between a left
  and a right curve is now undone, with the reason `short_straight`, as a window that breaks R_min
  at a join already is. Before, it was adopted and only a warning was given. An S-bend the starting
  line already had is kept and still reported. A minimum straight of 0 turns the check off.
