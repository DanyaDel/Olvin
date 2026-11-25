# Fault tolerance

The Linux principle: PipeWire or the window manager crashed, the kernel goes on, and you can fix things calmly. In Olvin the "kernel" is the loop, the ECS, the resources and the editor; the "services" are subsystem plugins (physics, audio, rendering, scripts, your own plugins). A crashed subsystem stops, writes a large report to the log and waits for a restart; everything else keeps running.

## What a panic does

Every system belongs to the plugin that added it (`App::add_plugin` remembers whose `build` is running; the `App`'s own systems belong to `core`). When a system panics:

1. **All** systems of that plugin stop, not just one function: a subsystem is never half alive.
2. A `Fault { owner, system, stage, message, frame }` goes into the `Faults` resource.
3. A `SUBSYSTEM CRASHED` block goes to the log with the subsystem, system, stage, frame, panic text and the list of stopped systems. With `Logging` installed, `logs/crash-*.txt` holds the backtrace.
4. The rest of the frame and the following frames run as usual.

Dependent subsystems may follow: if physics stopped in the middle of changing a resource, whoever reads it may trip. The log shows that as a chain of reports, like a cascade of failed units in systemd.

## Restarting

```rust
let mut faults = app.resources.fetch_mut::<Faults>();
for f in faults.list() {
    println!("{} crashed in {}: {}", f.owner, f.system, f.message);
}
faults.restart("PhysicsPlugin"); // or faults.restart_all()
```

`restart` turns the subsystem's systems back on from the next stage. `Startup` systems do not run again, and whatever state the panic left stays: fix it before restarting if it matters. `faults.down()` lists what is down now.

## Editor

- **Play**: the game is always isolated. A crashed game subsystem shows up as an error in the editor console; the game and the editor go on. Fix the code and press Play again.
- **Panels and elements**: a panel that panics is replaced by a "⚠ … crashed" card with the error and a **⟲ Restart panel** button (creates the panel anew). The rest of the editor works; the error is in the console.
- **Viewport extensions and frame hooks** of elements (the CSG and modeling tools and so on) are called one by one, guarded: a crashed one turns off only for its element, its tool falls back to View, and the scene view and other elements' tools keep working.
- **The Subsystems panel** (Window → Add Panel → 🛡 Subsystems), the editor's `systemctl status`: which game subsystems are down in Play (with a **⟲ Restart** button), the crash history (frame, subsystem, system, panic text) crashed editor panels, viewport extensions and element frame hooks, each with its own restart.
- **Autosave before Play**: when the scene has unsaved edits, pressing Play writes a copy to `<project>/.olvin/autosave/<scene path>`. The scene file is not touched. If the process still goes down (an abort, a driver reset), opening the scene next time asks "Recover Unsaved Changes?": Recover loads the copy as one undo step (undo returns the file's version), Discard deletes the copy, Later keeps it under File → Recover Autosave. The copy is deleted when the scene is saved and on "Don't Save".

## Strict mode

`OLVIN_FAULTS=abort` turns isolation off: the panic propagates and takes the process down as before. CI and `tools/quick_test.sh` use it, so an `assert!` inside a system fails the test instead of quietly stopping a subsystem. From code: `faults.set_policy(FaultPolicy::Abort)`, for the editor `EditorApp::set_fault_policy`.

## Not there yet

- Losing the GPU device (a driver reset) is not handled.
- Audio does not reconnect when the output device disappears during the game.
