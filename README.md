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

## What you can do

- **Your apps, or every light window.** Pick the apps to darken, or let Deonyx darken every light window and keep a list of programs it leaves alone.
- **Pick a look.** Gray is easy on the eyes, Black suits OLED screens, Soft lowers the contrast. A warm tint makes evenings easier.
- **Choose when.** Always, whenever Windows is in dark mode, or between the hours you set.
- **Keep real colors where you want them.** Draw a box over a video, a canvas or a 3D view and it stays as it is, even when the window is resized.
- **Ask why.** Click any window and Deonyx tells you why it is or isn't dark, with a button that fixes it when it can.
- **Hotkeys.** Ctrl+Alt+F darkens or restores the app in front of you, Ctrl+Alt+D turns darkening on and off.
- **Run as administrator** to darken apps like Task Manager, and **start with Windows** so it's always there.
- In English and Turkish, and it keeps itself up to date.

## Build

Needs Windows 11 (Windows 10 version 2004 or later should work but is untested) and [Rust](https://rustup.rs).

```
cargo build --release
target\release\deonyx.exe
```

Deonyx lives in the tray. Click its icon, choose **Add an app**, and pick a program from the open or installed ones (or browse for it). Each app has its own background darkness and text brightness. Settings are kept in `%APPDATA%\Deonyx\config.json`.

## License

Copyright (C) 2026 fdeox

Deonyx is free software under the GNU General Public License, version 3. See [LICENSE](LICENSE).

### Name and logo

The Deonyx name and logo are not covered by the GPL. They identify the official app made by fdeox. You are welcome to fork the code under the GPL, but a fork must use its own name and logo.
