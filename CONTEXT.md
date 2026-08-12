# niri-groom

An overlay that maps every workspace and window niri is running, so I can survey
them all at once and kill or rearrange them from the keyboard.

## Language

**Auto-show**:
A mode in which niri's focus follows the overlay's selection, so navigating the
map switches the real window underneath it.
_Avoid_: Follow mode, preview mode, live preview

**Output**:
One physical screen, as niri reports it. The canonical term in code, types and
prose. _Monitor_ survives only where it names something outside my control — a
niri action (`focus-monitor`, `move-workspace-to-monitor`) or a GTK type
(`gdk::Monitor`).
_Avoid_: Monitor, screen, display
