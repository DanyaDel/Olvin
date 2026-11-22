# Console and profiler

Two editor elements for debugging: **Console** (installed by default) and **Profiler** (`olvin-hub editor add profiler`).

## Console

The editor log panel: filters by level and text, **Copy** for the shown lines and **Copy line** for one. A record is one screen line; long and multi-line messages are truncated, the full text is in the tooltip. Up to 5000 lines are kept in memory. The editor log is also written to `<project>/logs/editor.log`.

## Profiler

The "⏱ Profiler" panel (**Window → Add Panel**):

- a frame time graph (60 and 30 fps lines) and a **Pause** button;
- render counters: Draw calls, Instances, Culled, Triangles (summed over editor views);
- a timeline of the frame's scopes (bars by depth);
- the **Systems and passes** table: name, ms, calls, from puffin frames.

Scope recording is on while the panel is open. `EditorApp::frame` has its own scope `editor frame`. The scopes themselves (systems, stages, rendering, physics, scripts) are described in [Profiler](../engine/profiler.md).

The statistics overlay in the viewport corner: Settings → Rendering → Editor views.

## See also

- [Profiler (engine)](../engine/profiler.md)
- [Slow frames](../troubleshooting/performance.md)
- [Scene view](scene-view.md)
- [Settings](settings.md)
