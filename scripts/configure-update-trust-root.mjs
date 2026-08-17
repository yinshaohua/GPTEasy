import { readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const [repository, publicKey, option, optionValue] = process.argv.slice(2);
if (
  !repository ||
  !publicKey ||
  (option && option !== "--repository-root") ||
  (option === "--repository-root" && !optionValue)
) {
  throw new Error(
    "usage: configure-update-trust-root.mjs <owner/repo> <public-key> [--repository-root <path>]",
  );
}
if (!/^[^/\s]+\/[^/\s]+$/.test(repository)) {
  throw new Error("GitCode repository must use owner/repo form");
}

let decodedKey;
try {
  decodedKey = Buffer.from(publicKey, "base64").toString("utf8");
} catch {
  throw new Error("updater public key is not valid base64");
}
const keyLines = decodedKey.trim().split(/\r?\n/);
if (
  !keyLines[0]?.startsWith("untrusted comment: ") ||
  !/\bminisign public key\b/.test(keyLines[0]) ||
  !/^RW[A-Za-z0-9+/]{54}$/.test(keyLines[1] ?? "")
) {
  throw new Error("updater public key is not a complete Tauri minisign public key");
}

const repositoryRoot = optionValue ? path.resolve(optionValue) : process.cwd();
const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
const distribution = JSON.parse(
  await readFile(path.join(scriptDirectory, "gitcode-distribution.json"), "utf8"),
);
const configPath = path.join(repositoryRoot, "src-tauri", "tauri.conf.json");
const config = JSON.parse(await readFile(configPath, "utf8"));
config.bundle ??= {};
config.bundle.createUpdaterArtifacts = true;
config.plugins ??= {};
config.plugins.updater = {
  endpoints: [
    `${distribution.rawBaseUrl}/${repository}/raw/${distribution.defaultBranch}/${distribution.formalManifestPath}`,
  ],
  pubkey: publicKey,
};
await writeFile(configPath, `${JSON.stringify(config, null, 2)}\n`, "utf8");

process.stdout.write(
  `${JSON.stringify({
    passed: true,
    endpoint: config.plugins.updater.endpoints[0],
    publicKeyConfigured: true,
  })}\n`,
);
