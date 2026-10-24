// Builds the Microsoft Store package from the release builds: an .msix per
// processor (x64, and arm64 when it's built) and the .msixbundle that goes
// to Partner Center. The Store signs it; nothing here needs a certificate.
//   cargo tauri build (and with --target aarch64-pc-windows-msvc), then
//   node store/pack.mjs
// identity.json holds what Partner Center > Product identity shows.

import { execFileSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..");
const out = join(root, "target", "store");
const identity = JSON.parse(readFileSync(join(here, "identity.json"), "utf8"));
const version = JSON.parse(readFileSync(join(root, "tauri.conf.json"), "utf8")).version + ".0";

// The newest Windows SDK's tools.
const kits = "C:\\Program Files (x86)\\Windows Kits\\10\\bin";
const sdk = readdirSync(kits).filter((v) => /^10\./.test(v) && existsSync(join(kits, v, "x64", "makeappx.exe"))).sort().at(-1);
if (!sdk) throw new Error("Windows SDK tools (makeappx) not found");
const tool = (name) => join(kits, sdk, "x64", name);
const run = (name, args) => {
  try {
    return execFileSync(tool(name), args, { stdio: ["ignore", "pipe", "pipe"] }).toString();
  } catch (e) {
    throw new Error(`${name} failed:
${e.stdout}${e.stderr}`);
  }
};

const builds = {
  x64: join(root, "target", "release", "deonyx.exe"),
  arm64: join(root, "target", "aarch64-pc-windows-msvc", "release", "deonyx.exe"),
};

// Only what this script makes; the Store screenshots live next to it.
for (const d of ["layout", "msix"]) rmSync(join(out, d), { recursive: true, force: true });
mkdirSync(join(out, "msix"), { recursive: true });
for (const [arch, exe] of Object.entries(builds)) {
  if (!existsSync(exe)) {
    console.log(`skip ${arch}: no ${exe}`);
    continue;
  }
  const layout = join(out, "layout", arch);
  mkdirSync(layout, { recursive: true });
  cpSync(exe, join(layout, "deonyx.exe"));
  cpSync(join(here, "Assets"), join(layout, "Assets"), { recursive: true });
  const manifest = readFileSync(join(here, "AppxManifest.xml"), "utf8")
    .replaceAll("{{NAME}}", identity.name)
    .replaceAll("{{PUBLISHER}}", identity.publisher)
    .replaceAll("{{PUBLISHER_DISPLAY_NAME}}", identity.publisherDisplayName)
    .replaceAll("{{VERSION}}", version)
    .replaceAll("{{ARCH}}", arch);
  writeFileSync(join(layout, "AppxManifest.xml"), manifest);
  // The images come in sizes and scales; resources.pri lets Windows pick.
  const priconfig = join(out, `priconfig-${arch}.xml`);
  run("makepri.exe", ["createconfig", "/cf", priconfig, "/dq", "en-US", "/pv", "10.0.0", "/o"]);
  run("makepri.exe", ["new", "/pr", layout, "/cf", priconfig, "/mn", join(layout, "AppxManifest.xml"), "/of", join(layout, "resources.pri"), "/o"]);
  const msix = join(out, "msix", `Deonyx_${version}_${arch}.msix`);
  run("makeappx.exe", ["pack", "/d", layout, "/p", msix, "/o"]);
  console.log(`packed ${msix}`);
}
const bundle = join(out, `Deonyx_${version}.msixbundle`);
run("makeappx.exe", ["bundle", "/d", join(out, "msix"), "/p", bundle, "/bv", version, "/o"]);
console.log(`bundled ${bundle}`);
