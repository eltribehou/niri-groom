# niri-groom

An overlay that maps every workspace and window niri is running, so I can survey
them all at once and kill or rearrange them from the keyboard.

## Language

**Auto-show**:
A mode in which niri's focus follows the overlay's selection, so navigating the
map switches the real window underneath it.
_Avoid_: Follow mode, preview mode, live preview

**Badge**:
The small pill drawn on a workspace card to show that the workspace carries a
mark. A rendering only — the state behind it lives outside the app.
_Avoid_: Pill, marker, flag

**Mark**:
A workspace's flagged state, keyed by the workspace name and stored outside the
app. An external command owns the store; the app reads it and toggles it.
_Avoid_: Favorite, tag, star

**Mark kind**:
One independent category of mark. Which kinds exist is configuration, so the app
never knows what any of them means.
_Avoid_: Type, class, group, category

**Output**:
One physical screen, as niri reports it. The canonical term in code, types and
prose. _Monitor_ survives only where it names something outside my control — a
niri action (`focus-monitor`, `move-workspace-to-monitor`) or a GTK type
(`gdk::Monitor`).
_Avoid_: Monitor, screen, display
