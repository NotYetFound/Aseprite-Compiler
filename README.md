# Aseprite Compiler

Compiles [Aseprite](https://www.aseprite.org/) from the official source and installs it on your computer — one button. It adds Aseprite to your app launcher, keeps it updated, and cleans up after itself.

> Not affiliated with [Igara Studio](https://igara.com/). No Aseprite binaries are distributed — everything is compiled locally from the official source, which the [Aseprite EULA](https://github.com/aseprite/aseprite/blob/main/EULA.txt) allows for personal use. If you enjoy Aseprite, and you're in a position to, please consider [buying](https://www.aseprite.org/) it and supporting the devs.
>
> Aseprite Compiler is made 100% with AI assistance — use with care, and open an [issue](https://github.com/NotYetFound/Aseprite-Compiler/issues/new) if you have any trouble.

## Install

Grab the latest from [Releases](https://github.com/NotYetFound/Aseprite-Compiler/releases):

- **Linux**: `Aseprite-Compiler_x.y.z_amd64.AppImage` — make it executable and run it.
- **Windows**: `Aseprite-Compiler_x.y.z_x64-setup.exe` — per-user install, no admin needed.

The app updates itself from this repository's releases.

## What you need

The app manages its own portable CMake and Ninja — nothing is installed system-wide. It only needs a compiler from your system, and the **Build tools** section in the app shows a ready-made install command if anything is missing:

- **Linux**: clang (or g++) and X11/OpenGL/Fontconfig headers
- **Windows**: Visual Studio 2022 Build Tools with the C++ workload

## Features

- One-click install, update, and rebuild of the latest Aseprite (stable or beta channel)
- Adds Aseprite to the app launcher / Start Menu
- Cleans up source and build files after installing (on by default, ~500 MB+ freed)
- Per-stage progress, resumable downloads, retry from the failed stage
- Atomic installs — a failed update keeps your previous build working
- Update notifications when a new Aseprite is out — when you launch Aseprite (via a launcher shim), optionally when a running Aseprite is detected, or on a schedule; building is always your click
- Launcher entries self-repair on app start, so a moved AppImage never leaves a broken shortcut
- Uninstall that removes the build and launcher entry, never your Aseprite files

## Build from source

```sh
npm install
npm run tauri dev
```

Release packages (Linux host; signing needs `src-tauri/updater.key`):

```sh
# Linux AppImage
NO_STRIP=true npm run tauri build

# Windows installer, cross-compiled from Linux
# needs: rustup target add x86_64-pc-windows-msvc · cargo install cargo-xwin · clang/lld · nsis
npm run tauri build -- --runner cargo-xwin --target x86_64-pc-windows-msvc --bundles nsis
```

CI builds both on every `v*` tag (`.github/workflows/release.yml`); it needs the signing key as the `TAURI_SIGNING_PRIVATE_KEY` repository secret.

Stack: Tauri 2 (Rust) + Svelte 5. Pipeline: resolve release → download source zip (submodules included, no git needed) → download the pinned prebuilt Skia → CMake + Ninja → install → register launcher → clean up.

## License

MIT — see [LICENSE](LICENSE). Aseprite itself is governed by its own [EULA](https://github.com/aseprite/aseprite/blob/main/EULA.txt).
