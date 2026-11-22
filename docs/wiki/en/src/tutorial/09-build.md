# Step 9. Building the game

Hand the game to someone without the editor and Cargo.

```sh
olvin-hub build --release
```

The hub runs `cargo build --release`, finds the exe and lays out the folder `dist/first_game/`:

```
dist/first_game/
  first_game          the executable
  assets/             a copy of the project's assets/ (without hidden files and *~)
  licenses/           licenses (Inter if the gui feature is on, and the project's)
```

Run it from any folder: `ProjectPaths` finds `assets/` next to the exe, and logs (`logs/`) and saves (`saves/`) are written there too. An archive: `olvin-hub build --release --archive tar.gz` (or `zip`) puts `first_game.tar.gz` alongside.

The hub appends `dist/` to `.gitignore` if the project is old and it is not there (the CLI says so). The output folder is rebuilt only if the hub created it (the `.olvin-build` marker); a foreign non-empty folder is left alone.

> **Status:** not done: cross-compilation (`--target`), asset compression and stripping, a GUI button, signing and installers. Asset packing: [#26](https://github.com/DanyaDel/Olvin/issues/26).

## What next

- Read [Editor: overview](../editor/overview.md) and [Engine](../engine/project-layout.md).
- Swap sounds and textures, add [materials](../editor/materials.md) and [animation](../editor/animation.md).
- If something went wrong: [FAQ](../troubleshooting/faq.md).

## See also

- [olvin-hub CLI: build](../reference/hub-cli.md#build)
- [Project layout](../engine/project-layout.md)
- [Hub](../start/hub.md)
- [Windows and macOS](../troubleshooting/platforms.md)
