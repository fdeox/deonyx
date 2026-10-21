// Bundles src/plugin.js with the Stream Deck SDK into the plugin folder, and
// with --pack zips that folder into a .streamDeckPlugin for the release.
import { build } from "esbuild";
import { execFileSync } from "node:child_process";
import { readFileSync, renameSync, rmSync } from "node:fs";
import { join } from "node:path";

const folder = "com.fdeox.deonyx.sdPlugin";

await build({
  entryPoints: ["src/plugin.js"],
  outfile: `${folder}/bin/plugin.mjs`,
  bundle: true,
  platform: "node",
  format: "esm",
  target: "node20",
  // ws asks for these speed-ups if they're there; they aren't.
  external: ["bufferutil", "utf-8-validate"],
  banner: { js: 'import { createRequire } from "node:module"; const require = createRequire(import.meta.url);' },
  legalComments: "none",
  logLevel: "info",
});

if (process.argv.includes("--pack")) {
  const { Version } = JSON.parse(readFileSync(`${folder}/manifest.json`, "utf8"));
  const out = `Deonyx-${Version.split(".").slice(0, 3).join(".")}.streamDeckPlugin`;
  rmSync(out, { force: true });
  // A .streamDeckPlugin is a zip with the plugin folder in it. Windows' own
  // tar writes zip paths with forward slashes (Compress-Archive doesn't).
  const tar = join(process.env.SystemRoot || "C:\\Windows", "System32", "tar.exe");
  execFileSync(tar, ["-a", "-c", "-f", `${out}.zip`, "--exclude", "logs", folder], { stdio: "inherit" });
  renameSync(`${out}.zip`, out);
  console.log(`packed ${out}`);
}
