// Builds the latest.json the in-app updater reads.
//
// Run it after `tauri build --bundles nsis` with TAURI_SIGNING_PRIVATE_KEY set
// (so the bundle step writes the .sig file), then upload BOTH the installer
// and latest.json to the GitHub release tagged v<version>.
//
//   node scripts/make-update-manifest.mjs [--notes "what changed"]

import { readFileSync, writeFileSync, readdirSync, mkdirSync, existsSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const version = JSON.parse(readFileSync(join(root, "tauri.conf.json"), "utf8")).version;
const repo = "fdeox/deonyx";

const bundleDir = join(root, "target/release/bundle/nsis");
if (!existsSync(bundleDir)) {
  console.error(`No bundle folder at ${bundleDir}. Build the installer first.`);
  process.exit(1);
}

// The exact version between underscores, so 0.1.0 never picks up a 0.1.0-beta.1.
const sigName = readdirSync(bundleDir).find((f) => f.includes(`_${version}_`) && f.endsWith(".exe.sig"));
if (!sigName) {
  console.error(`No .sig for ${version} in ${bundleDir}. Build with TAURI_SIGNING_PRIVATE_KEY set.`);
  process.exit(1);
}
const installer = sigName.replace(/\.sig$/, "");
const signature = readFileSync(join(bundleDir, sigName), "utf8").trim();

const at = process.argv.indexOf("--notes");
const notes = at !== -1 ? process.argv[at + 1] : `Deonyx ${version}`;

const manifest = {
  version,
  notes,
  pub_date: new Date().toISOString(),
  platforms: {
    "windows-x86_64": {
      signature,
      url: `https://github.com/${repo}/releases/download/v${version}/${encodeURIComponent(installer)}`,
    },
  },
};

const outDir = join(root, "releases");
mkdirSync(outDir, { recursive: true });
const out = join(outDir, "latest.json");
writeFileSync(out, JSON.stringify(manifest, null, 2));

console.log(`Wrote ${out}`);
console.log(`  version    ${version}`);
console.log(`  installer  ${installer}`);
console.log(`  url        ${manifest.platforms["windows-x86_64"].url}`);
console.log(`\nTag the release exactly v${version} and upload ${installer} and latest.json.`);
