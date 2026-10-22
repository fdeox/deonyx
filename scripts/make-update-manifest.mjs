// Builds the latest.json the in-app updater reads.
//
// Run it after `tauri build --bundles nsis` with TAURI_SIGNING_PRIVATE_KEY set
// (so the bundle step writes the .sig file), then upload the installers and
// latest.json to the GitHub release tagged v<version>. An ARM64 build
// (`--target aarch64-pc-windows-msvc`) is picked up too when it's there.
//
//   node scripts/make-update-manifest.mjs [--notes "what changed"]

import { readFileSync, writeFileSync, readdirSync, mkdirSync, existsSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const version = JSON.parse(readFileSync(join(root, "tauri.conf.json"), "utf8")).version;
const repo = "fdeox/deonyx";

// Where each kind of Windows finds its installer.
const builds = {
  "windows-x86_64": join(root, "target/release/bundle/nsis"),
  "windows-aarch64": join(root, "target/aarch64-pc-windows-msvc/release/bundle/nsis"),
};

const platforms = {};
for (const [platform, dir] of Object.entries(builds)) {
  if (!existsSync(dir)) continue;
  // The exact version between underscores, so 0.1.0 never picks up a 0.1.0-beta.1.
  const sigName = readdirSync(dir).find((f) => f.includes(`_${version}_`) && f.endsWith(".exe.sig"));
  if (!sigName) continue;
  const installer = sigName.replace(/\.sig$/, "");
  platforms[platform] = {
    signature: readFileSync(join(dir, sigName), "utf8").trim(),
    url: `https://github.com/${repo}/releases/download/v${version}/${encodeURIComponent(installer)}`,
  };
}
if (!platforms["windows-x86_64"]) {
  console.error(`No signed ${version} installer in ${builds["windows-x86_64"]}. Build with TAURI_SIGNING_PRIVATE_KEY set.`);
  process.exit(1);
}

const at = process.argv.indexOf("--notes");
const notes = at !== -1 ? process.argv[at + 1] : `Deonyx ${version}`;

const manifest = { version, notes, pub_date: new Date().toISOString(), platforms };

const outDir = join(root, "releases");
mkdirSync(outDir, { recursive: true });
const out = join(outDir, "latest.json");
writeFileSync(out, JSON.stringify(manifest, null, 2));

console.log(`Wrote ${out}`);
console.log(`  version    ${version}`);
for (const [platform, p] of Object.entries(platforms)) console.log(`  ${platform.padEnd(16)} ${p.url}`);
console.log(`\nTag the release exactly v${version} and upload the installers and latest.json.`);
