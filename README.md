<p align="center">
  <img src="assets/logo/deonyx.svg" width="128" alt="Deonyx logo">
</p>

<h1 align="center">Deonyx</h1>

<p align="center">Dark mode for any Windows app. Smart, flicker-free, and it keeps your 3D view's real colors.</p>

<p align="center"><a href="https://github.com/fdeox/deonyx/releases/latest"><b>Download for Windows</b></a></p>

## Why

Plenty of Windows apps still have no dark mode. SketchUp is one of them, and its users have been asking for one for years. The tools that fill the gap today are paid, and they simply invert colors: textures and pictures come out wrong, and the screen flashes white when you switch windows.

## How it works

- **Lightness, not colors.** Deonyx flips only how light each pixel is, measured the way the eye sees it (OKLab). White turns dark grey, black text turns soft white, and red stays red.
- **Your 3D view stays real.** Viewports, canvases and pictures are found automatically through Windows UI Automation and keep their original colors.
- **Every window of the app.** Menus, lists, tooltips and dialogs are darkened too, in less than a frame after they appear.
- **No flash.** The dark layer is tied to the app's own window, so switching windows never shows the light version.
- **Leaves dark things alone.** A window that is already dark is shown as it is.
- **Never touches the app.** Deonyx captures the window with Windows.Graphics.Capture and draws the result in a click-through layer on top of it. Nothing is injected into the app, and nothing it shows leaves your PC.

## What you can do

- **Your apps, or every light window.** Pick the apps to darken, or let Deonyx darken every light window and keep a list of programs it leaves alone.
- **Pick a look.** Gray is easy on the eyes, Black suits OLED screens, Soft lowers the contrast. A warm tint makes evenings easier.
- **Choose when.** Always, whenever Windows is in dark mode, or between the hours you set.
- **Keep real colors where you want them.** Draw a box over a video, a canvas or a 3D view and it stays as it is, even when the window is resized.
- **Ask why.** Click any window and Deonyx tells you why it is or isn't dark, with a button that fixes it when it can.
- **Hotkeys.** Ctrl+Alt+F darkens or restores the app in front of you, Ctrl+Alt+D turns darkening on and off.
- **Run as administrator** to darken apps like Task Manager, and **start with Windows** so it's always there. Normal apps are still darkened without administrator rights, so tools like AutoHotkey keep working in them.
- In 13 languages: English, Türkçe, Deutsch, Français, Español, Italiano, Português, Nederlands, Русский, Українська, 日本語, 한국어 and 简体中文. It keeps itself up to date.

## Install

Download `Deonyx_..._x64-setup.exe` from the [latest release](https://github.com/fdeox/deonyx/releases/latest) and run it. It installs for your user only, without administrator rights. Deonyx needs Windows 11, or Windows 10 version 2004 or later.

Deonyx isn't code-signed yet, so Windows SmartScreen may say it protected your PC. Click **More info**, then **Run anyway**.

Deonyx lives in the tray. Click its icon, choose **Add an app**, and pick a program from the open or installed ones (or browse for it). Each app has its own background darkness and text brightness. Settings are kept in `%APPDATA%\Deonyx\config.json`. To remove Deonyx, uninstall it from Settings > Apps like any other app.

## Privacy

Deonyx sends nothing anywhere. The only thing it asks the internet is whether a new version is out (from this repository's releases), and you can turn that off in its settings.

## Build

Needs [Rust](https://rustup.rs).

```
cargo build --release
target\release\deonyx.exe
```

The installer is built with the [Tauri CLI](https://v2.tauri.app/reference/cli/): `cargo tauri build --bundles nsis`.

## License

Copyright (C) 2026 fdeox

Deonyx is free software under the GNU General Public License, version 3. See [LICENSE](LICENSE).

### Name and logo

The Deonyx name and logo are not covered by the GPL. They identify the official app made by fdeox. You are welcome to fork the code under the GPL, but a fork must use its own name and logo.
