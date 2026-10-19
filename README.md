<p align="center">
  <img src="assets/logo/deonyx.svg" width="128" alt="Deonyx logo">
</p>

<h1 align="center">Deonyx</h1>

<p align="center">Dark mode for any Windows app. Smart, flicker-free, and it keeps your 3D view's real colors.</p>

> **Status:** early development. There is no release yet.

## Why

Plenty of Windows apps still have no dark mode. SketchUp is one of them, and its users have been asking for one for years. The tools that fill the gap today are paid, and they simply invert colors: textures and pictures come out wrong, and the screen flashes white when you switch windows.

## How it works

- **Lightness, not colors.** Deonyx flips only how light each pixel is, measured the way the eye sees it (OKLab). White turns dark grey, black text turns soft white, and red stays red.
- **Your 3D view stays real.** Viewports, canvases and pictures are found automatically through Windows UI Automation and keep their original colors.
- **Every window of the app.** Menus, lists, tooltips and dialogs are darkened too, in less than a frame after they appear.
- **No flash.** The dark layer is tied to the app's own window, so switching windows never shows the light version.
- **Leaves dark things alone.** A window that is already dark is shown as it is.
- **Never touches the app.** Deonyx captures the window with Windows.Graphics.Capture and draws the result in a click-through layer on top of it. Nothing is injected into the app, and nothing leaves your PC.

## Build

Needs Windows 11 (Windows 10 version 2004 or later should work but is untested) and [Rust](https://rustup.rs).

```
cargo build --release
target\release\deonyx.exe
```

Deonyx lives in the tray. Click its icon, choose **Add an app**, and pick a program from the list of open ones (or browse for it). Each app has its own background darkness and text brightness, and Ctrl+Alt+D turns darkening on and off from anywhere. Settings are kept in `%APPDATA%\Deonyx\config.json`.

## License

Copyright (C) 2026 fdeox

Deonyx is free software under the GNU General Public License, version 3. See [LICENSE](LICENSE).

### Name and logo

The Deonyx name and logo are not covered by the GPL. They identify the official app made by fdeox. You are welcome to fork the code under the GPL, but a fork must use its own name and logo.
