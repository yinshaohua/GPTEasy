import { readFile } from "node:fs/promises";

const [keyPath] = process.argv.slice(2);
if (!keyPath) {
  throw new Error("usage: check-encrypted-updater-key.mjs <private-key-path>");
}

const encoded = (await readFile(keyPath, "utf8")).trim();
const candidates = [encoded];
try {
  candidates.push(Buffer.from(encoded, "base64").toString("utf8"));
} catch {
  // The raw Tauri format is also accepted below.
}

const encrypted = candidates.some((candidate) => {
  const [header = ""] = candidate.split(/\r?\n/, 1);
  return (
    header.startsWith("untrusted comment: ") &&
    /\bencrypted secret key\b/.test(header)
  );
});

process.stdout.write(`${JSON.stringify({ encrypted })}\n`);
if (!encrypted) {
  process.exitCode = 1;
}
