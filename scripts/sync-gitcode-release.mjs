/* global Buffer, URL, fetch, process, setTimeout */

import { createHash } from "node:crypto";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";

const configuration = await loadConfiguration();
const workspace = await mkdtemp(path.join(os.tmpdir(), "gpteasy-gitcode-sync-"));
try {
  const release = await githubRequest(`/repos/${configuration.githubRepository}/releases/tags/${encodeURIComponent(configuration.tag)}`);
  validateRelease(release, configuration.tag);
  const currentManifest = await readCurrentManifest();
  if (!currentManifest) await verifyRawBaseline();
  if (currentManifest && compareVersions(release.tag_name.slice(1), currentManifest.manifest.version) < 0) {
    throw new Error(`refusing to replace newer manifest ${currentManifest.manifest.version} with ${release.tag_name.slice(1)}`);
  }
  const artifacts = selectArtifacts(release.assets ?? [], release.tag_name.slice(1));
  const downloaded = [];
  for (const asset of artifacts) {
    const target = path.join(workspace, asset.name);
    await downloadGithubAsset(asset, target, configuration.githubToken);
    const bytes = await readFile(target);
    downloaded.push({
      name: asset.name,
      path: target,
      bytes,
      size: bytes.length,
      sha256: sha256(bytes),
    });
  }

  const checksums = `${downloaded.map((asset) => `${asset.sha256}  ${asset.name}`).join("\n")}\n`;
  const checksumPath = path.join(workspace, "SHA256SUMS.txt");
  await writeFile(checksumPath, checksums, "utf8");
  downloaded.push({
    name: "SHA256SUMS.txt",
    path: checksumPath,
    bytes: Buffer.from(checksums),
    size: Buffer.byteLength(checksums),
    sha256: sha256(Buffer.from(checksums)),
  });

  const gitcodeRelease = await ensureGitcodeRelease(release, configuration);
  const verifiedAssets = [];
  for (const asset of downloaded) {
    const existing = findExistingAsset(gitcodeRelease, asset.name);
    let downloadUrl;
    if (existing) {
      downloadUrl = existing.download_url ?? existing.browser_download_url ?? attachmentUrl(configuration, asset.name);
      await verifyAnonymous(downloadUrl, asset);
    } else {
      const upload = await gitcodeRequest(
        `/repos/${configuration.gitcodeRepository}/releases/${encodeURIComponent(configuration.tag)}/upload_url?file_name=${encodeURIComponent(asset.name)}`,
        { method: "GET" },
      );
      if (!upload?.url || typeof upload.url !== "string") {
        throw new Error(`GitCode did not return an upload URL for ${asset.name}`);
      }
      await uploadAsset(upload, asset);
      downloadUrl = attachmentUrl(configuration, asset.name);
      await verifyAnonymous(downloadUrl, asset);
    }
    verifiedAssets.push({ ...asset, downloadUrl });
  }

  const installer = verifiedAssets.find((asset) => asset.name.toLowerCase().endsWith(".exe"));
  const signature = verifiedAssets.find((asset) => asset.name.toLowerCase().endsWith(".sig"));
  if (!installer || !signature) {
    throw new Error("GitHub Release must contain a Windows x64 installer and its updater signature");
  }
  const manifest = {
    version: release.tag_name.slice(1),
    notes: String(release.body ?? ""),
    pub_date: release.published_at,
    platforms: {
      "windows-x86_64": {
        url: installer.downloadUrl,
        signature: (await readFile(signature.path, "utf8")).trim(),
        sha256: installer.sha256,
        size: installer.size,
      },
    },
  };
  validateManifest(manifest);
  const manifestPath = configuration.formalManifestPath;
  const content = Buffer.from(`${JSON.stringify(manifest, null, 2)}\n`).toString("base64");
  // This is intentionally the final GitCode write. All network reads and asset
  // verification happen before it, so a failed sync leaves the previous manifest intact.
  await gitcodeRequest(`/repos/${configuration.gitcodeRepository}/contents/${manifestPath}`, {
    method: currentManifest ? "PUT" : "POST",
    body: JSON.stringify({
      branch: configuration.gitcodeBranch,
      message: `release: publish ${release.tag_name}`,
      content,
      ...(currentManifest?.sha ? { sha: currentManifest.sha } : {}),
    }),
  });
  process.stdout.write(`${JSON.stringify({
    passed: true,
    tag: release.tag_name,
    version: manifest.version,
    repository: configuration.gitcodeRepository,
    manifestPath,
    assets: verifiedAssets.map(({ name, size, sha256, downloadUrl }) => ({ name, size, sha256, downloadUrl })),
  })}\n`);
} finally {
  await rm(workspace, { recursive: true, force: true });
}

async function loadConfiguration() {
  const contract = JSON.parse(await readFile(new URL("./gitcode-distribution.json", import.meta.url), "utf8"));
  const required = (name) => {
    const value = process.env[name];
    if (!value) throw new Error(`${name} is required`);
    return value;
  };
  const tag = required("RELEASE_TAG");
  const githubRepository = required("GITHUB_REPOSITORY");
  const gitcodeRepository = required("GITCODE_REPOSITORY");
  const gitcodeToken = required("GITCODE_TOKEN");
  const githubToken = process.env.GITHUB_TOKEN ?? process.env.GH_TOKEN ?? required("GITHUB_TOKEN");
  if (!/^[^/\s]+\/[^/\s]+$/.test(githubRepository) || !/^[^/\s]+\/[^/\s]+$/.test(gitcodeRepository)) {
    throw new Error("GitHub and GitCode repositories must use owner/repo form");
  }
  if (!/^v(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)$/.test(tag)) {
    throw new Error("RELEASE_TAG must be a stable vSemVer tag");
  }
  const formalManifestPath = contract.formalManifestPath;
  if (!/^[^/\s]+\.md$/.test(formalManifestPath)) {
    throw new Error("formal manifest must be a single .md path");
  }
  const testMode = process.env.GITCODE_SYNC_TEST_MODE === "1";
  const githubApiBase = (process.env.GITHUB_API_URL ?? "https://api.github.com").replace(/\/$/, "");
  const gitcodeApiBase = (process.env.GITCODE_API_BASE_URL ?? contract.apiBaseUrl).replace(/\/$/, "");
  const gitcodeRawBase = (process.env.GITCODE_RAW_BASE_URL ?? contract.rawBaseUrl).replace(/\/$/, "");
  if (!testMode && (githubApiBase !== "https://api.github.com"
    || gitcodeApiBase !== "https://api.gitcode.com/api/v5"
    || gitcodeRawBase !== "https://raw.gitcode.com")) {
    throw new Error("custom API endpoints are test-only");
  }
  return {
    tag,
    githubRepository,
    githubToken,
    gitcodeRepository,
    gitcodeToken,
    gitcodeBranch: process.env.GITCODE_DEFAULT_BRANCH ?? contract.defaultBranch,
    githubApiBase,
    gitcodeApiBase,
    gitcodeRawBase,
    formalManifestPath,
    testMode,
    anonymousAttempts: testMode ? Number(process.env.GITCODE_SYNC_ANONYMOUS_ATTEMPTS ?? "1") : 40,
    anonymousDelayMs: testMode ? Number(process.env.GITCODE_SYNC_RETRY_DELAY_MS ?? "0") : 5000,
  };
}

async function githubRequest(endpoint) {
  const response = await fetch(`${configuration.githubApiBase}${endpoint}`, {
    headers: { Accept: "application/vnd.github+json", Authorization: `Bearer ${configuration.githubToken}` },
  });
  return parseJsonResponse(response, "GitHub");
}

async function gitcodeRequest(endpoint, options = {}) {
  const method = options.method ?? "GET";
  const response = await fetch(`${configuration.gitcodeApiBase}${endpoint}`, {
    ...options,
    headers: {
      Accept: "application/json",
      ...(options.body ? { "Content-Type": "application/json" } : {}),
      Authorization: `Bearer ${configuration.gitcodeToken}`,
      ...(options.headers ?? {}),
    },
  });
  if (response.status === 404 && method === "GET") return null;
  return parseJsonResponse(response, "GitCode");
}

async function parseJsonResponse(response, service) {
  const text = await response.text();
  if (!response.ok) throw new Error(`${service} request failed with HTTP ${response.status}: ${redact(text.slice(0, 300))}`);
  try { return text ? JSON.parse(text) : {}; } catch { throw new Error(`${service} returned invalid JSON`); }
}

function validateRelease(release, tag) {
  if (!release || release.draft || release.prerelease || release.tag_name !== tag || !isRfc3339(release.published_at)) {
    throw new Error("GitHub release must be a published stable release");
  }
}

async function readCurrentManifest() {
  const endpoint = `/repos/${configuration.gitcodeRepository}/contents/${configuration.formalManifestPath}?ref=${encodeURIComponent(configuration.gitcodeBranch)}`;
  const metadata = await gitcodeRequest(endpoint);
  if (!metadata) return null;
  if (typeof metadata.download_url !== "string") throw new Error("GitCode manifest metadata is missing its anonymous download URL");
  const url = new URL(metadata.download_url);
  const validOrigin = url.origin === new URL(configuration.gitcodeRawBase).origin;
  const validTestUrl = configuration.testMode && url.protocol === "http:" && url.hostname === "127.0.0.1";
  if (!validOrigin && !validTestUrl) throw new Error("GitCode returned an unexpected manifest download URL");
  const response = await fetchAnonymousWithRetry(url, "Anonymous current manifest download");
  let manifest;
  try { manifest = JSON.parse(await response.text()); } catch { throw new Error("current manifest is not valid JSON"); }
  validateManifest(manifest);
  return { manifest, sha: typeof metadata.sha === "string" ? metadata.sha : undefined };
}

async function verifyRawBaseline() {
  const endpoint = `/repos/${configuration.gitcodeRepository}/contents/README.md?ref=${encodeURIComponent(configuration.gitcodeBranch)}`;
  const metadata = await gitcodeRequest(endpoint);
  if (!metadata || typeof metadata.download_url !== "string") {
    throw new Error("GitCode distribution README metadata is unavailable");
  }
  const url = new URL(metadata.download_url);
  const validOrigin = url.origin === new URL(configuration.gitcodeRawBase).origin;
  const validTestUrl = configuration.testMode && url.protocol === "http:" && url.hostname === "127.0.0.1";
  if (!validOrigin && !validTestUrl) throw new Error("GitCode returned an unexpected README download URL");
  const response = await fetchAnonymousWithRetry(url, "Anonymous GitCode Raw baseline download");
  if (!(await response.text()).trim()) throw new Error("Anonymous GitCode Raw baseline download returned an empty response");
}

function selectArtifacts(assets, version) {
  const expectedInstaller = `GPTEasy_${version}_x64-setup.exe`;
  const installer = assets.find((asset) => asset.name.toLowerCase() === expectedInstaller.toLowerCase());
  const signature = assets.find((asset) => asset.name.toLowerCase() === `${installer?.name.toLowerCase()}.sig`);
  if (!installer || !signature) {
    throw new Error(`Release assets must include ${expectedInstaller} and its .sig`);
  }
  return [installer, signature];
}

async function downloadGithubAsset(asset, target, token) {
  const response = await fetch(asset.url ?? asset.browser_download_url, {
    headers: { Accept: "application/octet-stream", Authorization: `Bearer ${token}` },
  });
  if (!response.ok) throw new Error(`GitHub asset ${asset.name} download failed with HTTP ${response.status}`);
  await writeFile(target, Buffer.from(await response.arrayBuffer()));
}

async function ensureGitcodeRelease(release, config) {
  const endpoint = `/repos/${config.gitcodeRepository}/releases/${encodeURIComponent(config.tag)}`;
  const existing = await gitcodeRequest(endpoint);
  const expected = { tag_name: config.tag, name: release.name ?? config.tag, body: release.body ?? "" };
  if (existing) {
    if (existing.tag_name && existing.tag_name !== config.tag) {
      throw new Error("GitCode returned a release with a different tag");
    }
    if (existing.name !== expected.name || existing.body !== expected.body) {
      const updated = await gitcodeRequest(endpoint, { method: "PATCH", body: JSON.stringify(expected) });
      return { ...existing, ...updated };
    }
    return existing;
  }
  return gitcodeRequest(`/repos/${config.gitcodeRepository}/releases`, {
    method: "POST",
    body: JSON.stringify(expected),
  });
}

function findExistingAsset(release, name) {
  const assets = [release?.assets, release?.attach_files, release?.attachments].flatMap((value) => Array.isArray(value) ? value : []);
  return assets.find((asset) => asset.name === name || asset.file_name === name);
}

async function uploadAsset(upload, asset) {
  const url = new URL(upload.url);
  if (url.protocol !== "https:" && !(configuration.testMode && url.protocol === "http:" && url.hostname === "127.0.0.1")) {
    throw new Error("GitCode returned an invalid attachment upload URL");
  }
  const response = await fetch(upload.url, {
    method: "PUT",
    headers: upload.headers,
    body: asset.bytes,
    redirect: "follow",
  });
  if (!response.ok) throw new Error(`GitCode upload failed for ${asset.name} with HTTP ${response.status}`);
}

function attachmentUrl(config, name) {
  return `${config.gitcodeApiBase}/repos/${config.gitcodeRepository}/releases/${encodeURIComponent(config.tag)}/attach_files/${encodeURIComponent(name)}/download`;
}

async function verifyAnonymous(url, asset) {
  let lastStatus = 0;
  for (let attempt = 1; attempt <= configuration.anonymousAttempts; attempt += 1) {
    const response = await fetch(url, { redirect: "follow" });
    lastStatus = response.status;
    if (response.ok) {
      const bytes = Buffer.from(await response.arrayBuffer());
      if (bytes.length !== asset.size || sha256(bytes) !== asset.sha256) {
        throw new Error(`GitCode attachment ${asset.name} does not match the GitHub Release bytes`);
      }
      return;
    }
    if (attempt < configuration.anonymousAttempts && [403, 404, 418, 429, 500, 502, 503, 504].includes(response.status)) {
      await new Promise((resolve) => setTimeout(resolve, configuration.anonymousDelayMs));
      continue;
    }
    break;
  }
  throw new Error(`Anonymous download failed for ${asset.name} with HTTP ${lastStatus}`);
}

async function fetchAnonymousWithRetry(url, description) {
  let lastStatus = 0;
  for (let attempt = 1; attempt <= configuration.anonymousAttempts; attempt += 1) {
    const response = await fetch(url, { redirect: "follow" });
    lastStatus = response.status;
    if (response.ok) return response;
    if (attempt < configuration.anonymousAttempts && isRetryableAnonymousStatus(response.status)) {
      await new Promise((resolve) => setTimeout(resolve, configuration.anonymousDelayMs));
      continue;
    }
    break;
  }
  throw new Error(`${description} failed with HTTP ${lastStatus}`);
}

function isRetryableAnonymousStatus(status) {
  return [403, 404, 418, 429, 500, 502, 503, 504].includes(status);
}

function validateManifest(manifest) {
  if (!/^\d+\.\d+\.\d+$/.test(manifest.version) || !isRfc3339(manifest.pub_date) || !manifest.platforms?.["windows-x86_64"]
    || Object.keys(manifest.platforms).length !== 1) {
    throw new Error("formal update manifest is incomplete");
  }
  const entry = manifest.platforms["windows-x86_64"];
  const validUrl = entry.url.startsWith("https://")
    || (configuration.testMode && entry.url.startsWith("http://127.0.0.1:"));
  if (!validUrl || !isUpdaterSignature(entry.signature) || !/^[a-f0-9]{64}$/.test(entry.sha256) || !entry.size) {
    throw new Error("formal update manifest contains invalid Windows x64 data");
  }
}

function isUpdaterSignature(value) {
  if (typeof value !== "string" || /^https?:\/\//i.test(value)) return false;
  try {
    const decoded = Buffer.from(value, "base64").toString("utf8");
    return decoded.startsWith("untrusted comment: ") && decoded.split(/\r?\n/).length >= 4;
  } catch {
    return false;
  }
}

function compareVersions(left, right) {
  const a = left.split(".").map(Number);
  const b = right.split(".").map(Number);
  for (let index = 0; index < 3; index += 1) {
    if (a[index] !== b[index]) return a[index] - b[index];
  }
  return 0;
}

function isRfc3339(value) {
  return typeof value === "string"
    && /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$/.test(value)
    && !Number.isNaN(Date.parse(value));
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function redact(value) {
  return String(value)
    .replaceAll(configuration.gitcodeToken, "<REDACTED>")
    .replaceAll(configuration.githubToken, "<REDACTED>");
}
