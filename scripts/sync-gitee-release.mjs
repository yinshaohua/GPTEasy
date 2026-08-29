/* global AbortSignal, Buffer, URL, fetch, process, setTimeout */

import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";

const configuration = await loadConfiguration();
const workspace = await mkdtemp(path.join(os.tmpdir(), "gpteasy-Gitee-sync-"));
try {
  const release = await githubRequest(`/repos/${configuration.githubRepository}/releases/tags/${encodeURIComponent(configuration.tag)}`);
  validateRelease(release, configuration.tag);
  // GitHub API preserves a body accidentally submitted as literal "\\n" text.
  // Normalize it before copying release notes to Gitee and the updater manifest.
  release.body = normalizeReleaseBody(release.body);
  const currentManifest = await readCurrentManifest();
  await ensureGiteeReadme();
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
      distributionName: giteeAssetName(asset.name),
      path: target,
      bytes,
      size: bytes.length,
      sha256: sha256(bytes),
    });
  }

  const checksums = `${downloaded.map((asset) => `${asset.sha256}  ${asset.distributionName}`).join("\n")}\n`;
  const checksumPath = path.join(workspace, "SHA256SUMS.txt");
  await writeFile(checksumPath, checksums, "utf8");
  downloaded.push({
    name: "SHA256SUMS.txt",
    distributionName: "SHA256SUMS.txt",
    path: checksumPath,
    bytes: Buffer.from(checksums),
    size: Buffer.byteLength(checksums),
    sha256: sha256(Buffer.from(checksums)),
  });

  const giteeRelease = await ensureGiteeRelease(release, configuration);
  const verifiedAssets = [];
  for (const asset of downloaded) {
    const existing = findExistingAsset(giteeRelease, asset.distributionName);
    let downloadUrl;
    if (existing) {
      downloadUrl = stableAttachmentUrl(configuration, numericReleaseId(giteeRelease), existing, asset.distributionName);
      await verifyAnonymous(downloadUrl, asset);
    } else {
      const releaseId = numericReleaseId(giteeRelease);
      const distributionAsset = { ...asset, name: asset.distributionName };
      const uploaded = await uploadAsset(releaseId, distributionAsset);
      downloadUrl = stableAttachmentUrl(configuration, releaseId, uploaded, asset.distributionName);
      await verifyAnonymous(downloadUrl, distributionAsset);
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
  // This is intentionally the final Gitee write. All network reads and asset
  // verification happen before it, so a failed sync leaves the previous manifest intact.
  await giteeRequest(`/repos/${configuration.giteeRepository}/contents/${manifestPath}`, {
    method: currentManifest ? "PUT" : "POST",
    body: urlEncodedForm({
      branch: configuration.giteeBranch,
      message: `release: publish ${release.tag_name}`,
      content,
      ...(currentManifest?.sha ? { sha: currentManifest.sha } : {}),
    }),
  });
  process.stdout.write(`${JSON.stringify({
    passed: true,
    tag: release.tag_name,
    version: manifest.version,
    repository: configuration.giteeRepository,
    manifestPath,
    assets: verifiedAssets.map(({ name, distributionName, size, sha256, downloadUrl }) => ({
      sourceName: name,
      name: distributionName,
      size,
      sha256,
      downloadUrl,
    })),
  })}\n`);
} finally {
  await rm(workspace, { recursive: true, force: true });
}

async function loadConfiguration() {
  const contract = JSON.parse(await readFile(new URL("./gitee-distribution.json", import.meta.url), "utf8"));
  const required = (name) => {
    const value = process.env[name];
    if (!value) throw new Error(`${name} is required`);
    return value;
  };
  const tag = required("RELEASE_TAG");
  const githubRepository = required("GITHUB_REPOSITORY");
  const giteeRepository = required("GITEE_REPOSITORY");
  const giteeToken = required("GITEE_TOKEN");
  const githubToken = process.env.GITHUB_TOKEN ?? process.env.GH_TOKEN ?? required("GITHUB_TOKEN");
  if (!/^[^/\s]+\/[^/\s]+$/.test(githubRepository) || !/^[^/\s]+\/[^/\s]+$/.test(giteeRepository)) {
    throw new Error("GitHub and Gitee repositories must use owner/repo form");
  }
  if (!/^v(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)$/.test(tag)) {
    throw new Error("RELEASE_TAG must be a stable vSemVer tag");
  }
  const formalManifestPath = contract.formalManifestPath;
  if (!/^[^/\s]+\.md$/.test(formalManifestPath)) {
    throw new Error("formal manifest must be a single .md path");
  }
  const testMode = process.env.GITEE_SYNC_TEST_MODE === "1";
  const githubApiBase = (process.env.GITHUB_API_URL ?? "https://api.github.com").replace(/\/$/, "");
  const giteeApiBase = (process.env.GITEE_API_BASE_URL ?? contract.apiBaseUrl).replace(/\/$/, "");
  const giteeRawBase = (process.env.GITEE_RAW_BASE_URL ?? contract.rawBaseUrl).replace(/\/$/, "");
  if (!testMode && (githubApiBase !== "https://api.github.com"
    || giteeApiBase !== "https://gitee.com/api/v5"
    || giteeRawBase !== "https://gitee.com")) {
    throw new Error("custom API endpoints are test-only");
  }
  return {
    tag,
    githubRepository,
    githubToken,
    giteeRepository,
    giteeToken,
    giteeBranch: process.env.GITEE_DEFAULT_BRANCH ?? contract.defaultBranch,
    githubApiBase,
    giteeApiBase,
    giteeRawBase,
    formalManifestPath,
    testMode,
    requestAttempts: testMode ? Number(process.env.GITEE_SYNC_REQUEST_ATTEMPTS ?? "3") : 3,
    requestDelayMs: testMode ? Number(process.env.GITEE_SYNC_RETRY_DELAY_MS ?? "0") : 5000,
    requestTimeoutMs: testMode ? Number(process.env.GITEE_SYNC_REQUEST_TIMEOUT_MS ?? "1000") : 30000,
    anonymousAttempts: testMode ? Number(process.env.GITEE_SYNC_ANONYMOUS_ATTEMPTS ?? "1") : 40,
    anonymousDelayMs: testMode ? Number(process.env.GITEE_SYNC_RETRY_DELAY_MS ?? "0") : 5000,
    uploadAttempts: testMode ? Number(process.env.GITEE_SYNC_UPLOAD_ATTEMPTS ?? "3") : 3,
    uploadDelayMs: testMode ? Number(process.env.GITEE_SYNC_RETRY_DELAY_MS ?? "0") : 5000,
    uploadTimeoutMs: testMode ? Number(process.env.GITEE_SYNC_UPLOAD_TIMEOUT_MS ?? "1000") : 300000,
  };
}

async function githubRequest(endpoint) {
  const response = await fetchWithRetry(`${configuration.githubApiBase}${endpoint}`, {
    headers: { Accept: "application/vnd.github+json", Authorization: `Bearer ${configuration.githubToken}` },
  }, "GitHub");
  return parseJsonResponse(response, "GitHub");
}

async function giteeRequest(endpoint, options = {}) {
  const method = options.method ?? "GET";
  const response = await fetchWithRetry(`${configuration.giteeApiBase}${endpoint}`, {
    ...options,
    headers: {
      Accept: "application/json",
      ...(options.body && !(options.body instanceof URLSearchParams) && !(options.body instanceof FormData)
        ? { "Content-Type": "application/json" } : {}),
      Authorization: `Bearer ${configuration.giteeToken}`,
      ...(options.headers ?? {}),
    },
  }, "Gitee", method === "GET");
  if (response.status === 404 && method === "GET") return null;
  return parseJsonResponse(response, "Gitee");
}

async function fetchWithRetry(url, options, service, allowNotFound = false) {
  const method = options.method ?? "GET";
  const retryable = ["GET", "PATCH", "PUT"].includes(method);
  let lastError;
  for (let attempt = 1; attempt <= configuration.requestAttempts; attempt += 1) {
    let retryStatus;
    try {
      const response = await fetch(url, {
        ...options,
        signal: options.signal ?? AbortSignal.timeout(configuration.requestTimeoutMs),
      });
      if (allowNotFound && response.status === 404) return response;
      if (response.ok || !retryable || attempt >= configuration.requestAttempts || !isRetryableApiStatus(response.status)) {
        return response;
      }
      retryStatus = `HTTP-${response.status}`;
      await response.arrayBuffer();
    } catch (error) {
      lastError = error;
      if (!retryable || attempt >= configuration.requestAttempts) throw error;
      retryStatus = "network-error";
    }
    writeRetryLog(service, method, "api", retryStatus, attempt, configuration.requestAttempts);
    await new Promise((resolve) => setTimeout(resolve, configuration.requestDelayMs));
  }
  throw lastError;
}

function writeRetryLog(service, method, stage, status, attempt, total) {
  process.stderr.write(
    `retry service=${service} method=${method} stage=${stage} status=${status} attempt=${attempt}/${total}\n`,
  );
}

function isRetryableApiStatus(status) {
  return [408, 425, 429, 500, 502, 503, 504].includes(status);
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

function normalizeReleaseBody(body) {
  return String(body ?? "")
    .replaceAll("\\r\\n", "\n")
    .replaceAll("\\n", "\n")
    .replaceAll("\\r", "\r");
}

async function readCurrentManifest() {
  const endpoint = `/repos/${configuration.giteeRepository}/contents/${configuration.formalManifestPath}?ref=${encodeURIComponent(configuration.giteeBranch)}`;
  const metadata = await giteeRequest(endpoint);
  if (!metadata || (Array.isArray(metadata) && metadata.length === 0)) return null;
  const embedded = decodeApiContent(metadata);
  if (embedded !== null) {
    const manifest = parseManifest(embedded);
    validateManifest(manifest);
    return { manifest, sha: typeof metadata.sha === "string" ? metadata.sha : undefined };
  }
  if (typeof metadata.download_url !== "string") throw new Error("Gitee manifest metadata is missing its anonymous download URL");
  const url = new URL(metadata.download_url);
  const validOrigin = url.origin === new URL(configuration.giteeRawBase).origin;
  const validTestUrl = configuration.testMode && url.protocol === "http:" && url.hostname === "127.0.0.1";
  if (!validOrigin && !validTestUrl) throw new Error("Gitee returned an unexpected manifest download URL");
  const branchUrl = branchRawUrl(configuration.formalManifestPath);
  const content = await fetchAnonymousFromCandidates(
    [branchUrl, url],
    "Anonymous current manifest download",
    (response) => response.text(),
  );
  const manifest = parseManifest(content);
  validateManifest(manifest);
  return { manifest, sha: typeof metadata.sha === "string" ? metadata.sha : undefined };
}

function decodeApiContent(metadata) {
  if (typeof metadata?.content !== "string") return null;
  if (metadata.encoding && metadata.encoding !== "base64") {
    throw new Error(`Gitee manifest content uses unsupported encoding ${metadata.encoding}`);
  }
  try {
    return Buffer.from(metadata.content.replace(/\s/g, ""), "base64").toString("utf8");
  } catch {
    throw new Error("Gitee manifest API content is not valid Base64");
  }
}

function parseManifest(content) {
  try { return JSON.parse(content); } catch { throw new Error("current manifest is not valid JSON"); }
}

async function ensureGiteeReadme() {
  const expected = await readFile(new URL("../README.md", import.meta.url), "utf8");
  const endpoint = `/repos/${configuration.giteeRepository}/contents/README.md?ref=${encodeURIComponent(configuration.giteeBranch)}`;
  const metadata = await giteeRequest(endpoint);
  if (!metadata || typeof metadata.sha !== "string" || typeof metadata.download_url !== "string") {
    throw new Error("Gitee distribution README metadata is unavailable");
  }
  const current = await readAnonymousGiteeText(
    rawUrlWithRevision(metadata.download_url, metadata.sha),
    "Anonymous Gitee README download",
  );
  if (current === expected) return;

  const updated = await giteeRequest(`/repos/${configuration.giteeRepository}/contents/README.md`, {
    method: "PUT",
    body: urlEncodedForm({
      branch: configuration.giteeBranch,
      message: "docs: update GPTEasy download instructions",
      content: Buffer.from(expected).toString("base64"),
      sha: metadata.sha,
    }),
  });
  const downloadUrl = updated?.content?.download_url ?? updated?.download_url;
  const revision = updated?.content?.sha;
  if (typeof downloadUrl !== "string" || typeof revision !== "string") {
    throw new Error("Gitee README update response is missing its anonymous download URL");
  }
  const published = await readAnonymousGiteeText(
    rawUrlWithRevision(downloadUrl, revision),
    "Anonymous updated Gitee README download",
  );
  if (published !== expected) throw new Error("Anonymous updated Gitee README does not match the repository root README");
}

function rawUrlWithRevision(downloadUrl, revision) {
  const url = new URL(downloadUrl);
  url.searchParams.set("gpteasy_sha", revision);
  return url.toString();
}

async function readAnonymousGiteeText(downloadUrl, label) {
  const url = new URL(downloadUrl);
  const validOrigin = url.origin === new URL(configuration.giteeRawBase).origin;
  const validTestUrl = configuration.testMode && url.protocol === "http:" && url.hostname === "127.0.0.1";
  if (!validOrigin && !validTestUrl) throw new Error("Gitee returned an unexpected README download URL");
  return fetchAnonymousFromCandidates(
    [url, branchRawUrl("README.md")],
    label,
    (response) => response.text(),
  );
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

function giteeAssetName(name) {
  return name.toLowerCase().endsWith(".exe") ? `${name}.bin` : name;
}

async function downloadGithubAsset(asset, target, token) {
  const response = await fetchWithRetry(asset.url ?? asset.browser_download_url, {
    headers: { Accept: "application/octet-stream", Authorization: `Bearer ${token}` },
  }, "GitHub");
  if (!response.ok) throw new Error(`GitHub asset ${asset.name} download failed with HTTP ${response.status}`);
  await writeFile(target, Buffer.from(await response.arrayBuffer()));
}

async function ensureGiteeRelease(release, config) {
  const endpoint = `/repos/${config.giteeRepository}/releases/tags/${encodeURIComponent(config.tag)}`;
  const existing = await giteeRequest(endpoint);
  const expected = {
    tag_name: config.tag,
    name: release.name ?? config.tag,
    body: giteeReleaseBody(release.body),
    prerelease: false,
  };
  if (existing) {
    if (existing.tag_name && existing.tag_name !== config.tag) {
      throw new Error("Gitee returned a release with a different tag");
    }
    if (existing.name !== expected.name || existing.body !== expected.body) {
      const updated = await giteeRequest(`/repos/${config.giteeRepository}/releases/${numericReleaseId(existing)}`, { method: "PATCH", body: urlEncodedForm(expected) });
      return { ...existing, ...updated };
    }
    return existing;
  }
  return giteeRequest(`/repos/${config.giteeRepository}/releases`, {
    method: "POST",
    body: urlEncodedForm({ ...expected, target_commitish: config.giteeBranch }),
  });
}

function giteeReleaseBody(body) {
  const notice = [
    "## Gitee 下载说明",
    "",
    "Gitee 上的 Windows 安装包因平台附件限制使用 `.exe.bin` 后缀。手工下载后请删除末尾 `.bin`，再按 `SHA256SUMS.txt` 核对后运行；应用内更新无需手工处理。",
  ].join("\n");
  const normalized = String(body ?? "").trimEnd();
  return normalized ? `${normalized}\n\n${notice}` : notice;
}

function findExistingAsset(release, name) {
  const assets = [release?.assets, release?.attach_files, release?.attachments].flatMap((value) => Array.isArray(value) ? value : []);
  return assets.find((asset) => asset.name === name || asset.file_name === name);
}

async function uploadAsset(releaseId, asset) {
  const endpoint = `${configuration.giteeApiBase}/repos/${configuration.giteeRepository}/releases/${releaseId}/attach_files`;
  const url = new URL(endpoint);
  if (url.protocol !== "https:" && !(configuration.testMode && url.protocol === "http:" && url.hostname === "127.0.0.1")) {
    throw new Error("Gitee returned an invalid attachment upload URL");
  }
  for (let attempt = 1; attempt <= configuration.uploadAttempts; attempt += 1) {
    let retryStatus = "network-error";
    let failure;
    try {
      const response = await curlUpload(endpoint, asset);
      if (response.status >= 200 && response.status < 300) {
        try {
          return JSON.parse(response.body);
        } catch (error) {
          failure = new Error(`Gitee upload returned invalid JSON for ${asset.name}`, { cause: error });
        }
      } else if (!isRetryableUploadStatus(response.status)) {
        throw new Error(`Gitee upload failed for ${asset.name} with HTTP ${response.status}`);
      } else {
        retryStatus = `HTTP-${response.status}`;
        failure = new Error(`Gitee upload failed for ${asset.name} with HTTP ${response.status}`);
      }
    } catch (error) {
      if (error instanceof Error && error.message.startsWith("Gitee upload failed")
        && !/HTTP (408|425|429|500|502|503|504)$/.test(error.message)) {
        throw error;
      }
      failure = error;
    }

    const reconciled = await reconcileUploadedAsset(releaseId, asset, attempt);
    if (reconciled) return reconciled;
    if (attempt >= configuration.uploadAttempts) {
      throw failure;
    }
    writeAssetRetryLog(asset, retryStatus, attempt, configuration.uploadAttempts);
    await new Promise((resolve) => setTimeout(resolve, configuration.uploadDelayMs));
  }
}

async function reconcileUploadedAsset(releaseId, asset, attempt) {
  try {
    const attachments = await giteeRequest(
      `/repos/${configuration.giteeRepository}/releases/${releaseId}/attach_files?per_page=100`,
    );
    const existing = Array.isArray(attachments)
      ? attachments.find((candidate) => candidate.name === asset.name || candidate.file_name === asset.name)
      : undefined;
    if (!existing) return undefined;
    process.stderr.write(
      `recovery service=Gitee method=GET stage=attachment-reconcile status=found asset=${asset.name} size=${asset.size} attempt=${attempt}/${configuration.uploadAttempts}\n`,
    );
    return existing;
  } catch (error) {
    process.stderr.write(
      `recovery service=Gitee method=GET stage=attachment-reconcile status=lookup-error asset=${asset.name} size=${asset.size} attempt=${attempt}/${configuration.uploadAttempts}\n`,
    );
    return undefined;
  }
}

function writeAssetRetryLog(asset, status, attempt, total) {
  process.stderr.write(
    `retry service=Gitee method=POST stage=attachment-upload status=${status} asset=${asset.name} size=${asset.size} attempt=${attempt}/${total}\n`,
  );
}

async function curlUpload(endpoint, asset) {
  const timeoutSeconds = Math.max(1, Math.ceil(configuration.uploadTimeoutMs / 1000));
  // Gitee accepts the smoke-tested curl multipart request but stalls on Node fetch FormData uploads.
  const child = spawn("curl", [
    "--silent",
    "--show-error",
    "--location",
    "--http1.1",
    "--request", "POST",
    "--header", "Accept: application/json",
    "--header", "Expect:",
    "--header", "@-",
    "--form", `file=@${asset.path};filename=${asset.name};type=application/octet-stream`,
    "--max-time", String(timeoutSeconds),
    "--output", "-",
    "--write-out", "\n%{http_code}",
    endpoint,
  ], { stdio: ["pipe", "pipe", "pipe"], windowsHide: true });

  let stdout = "";
  let stderr = "";
  child.stdout.setEncoding("utf8").on("data", (chunk) => { stdout += chunk; });
  child.stderr.setEncoding("utf8").on("data", (chunk) => { stderr += chunk; });
  child.stdin.end(`Authorization: Bearer ${configuration.giteeToken}\n`);

  const code = await new Promise((resolve, reject) => {
    child.on("error", reject);
    child.on("close", resolve);
  });
  if (code !== 0) {
    const detail = redact(stderr.trim().slice(0, 300));
    throw new Error(`Gitee upload transport failed for ${asset.name} with curl exit ${code}${detail ? `: ${detail}` : ""}`);
  }
  const match = stdout.match(/\n(\d{3})$/);
  if (!match) throw new Error(`Gitee upload returned no HTTP status for ${asset.name}`);
  return { status: Number(match[1]), body: stdout.slice(0, -match[0].length) };
}

function isRetryableUploadStatus(status) {
  return [408, 425, 429, 500, 502, 503, 504].includes(status);
}

function numericReleaseId(release) {
  const id = Number(release?.id);
  if (configuration.testMode && (!Number.isSafeInteger(id) || id <= 0)) return 1;
  if (!Number.isSafeInteger(id) || id <= 0) throw new Error("Gitee release response is missing a numeric release ID");
  return id;
}

function stableAttachmentUrl(config, releaseId, uploaded, name) {
  const id = Number(uploaded?.id);
  if (!Number.isSafeInteger(id) || id <= 0) throw new Error(`Gitee upload response is missing a numeric attachment ID for ${name}`);
  const candidate = uploaded.browser_download_url ?? uploaded.download_url;
  if (typeof candidate === "string") {
    const url = new URL(candidate);
    const validTestUrl = configuration.testMode && url.protocol === "http:" && url.hostname === "127.0.0.1";
    if ((!validTestUrl && url.protocol !== "https:") || url.search) throw new Error(`Gitee returned an unstable attachment URL for ${name}`);
    return candidate;
  }
  return `${config.giteeRawBase}/${config.giteeRepository}/releases/download/${encodeURIComponent(config.tag)}/${encodeURIComponent(name)}`;
}

function urlEncodedForm(values) {
  const body = new URLSearchParams();
  for (const [key, value] of Object.entries(values)) body.set(key, String(value));
  return body;
}

async function verifyAnonymous(url, asset) {
  let lastStatus = 0;
  for (let attempt = 1; attempt <= configuration.anonymousAttempts; attempt += 1) {
    let response;
    try {
      response = await fetch(url, {
        redirect: "follow",
        headers: { "user-agent": "Mozilla/5.0 (compatible; GPTEasy release sync)" },
        signal: AbortSignal.timeout(configuration.requestTimeoutMs),
      });
    } catch (error) {
      if (attempt >= configuration.anonymousAttempts) throw error;
      writeRetryLog(
        "Gitee",
        "GET",
        "attachment-download",
        "network-error",
        attempt,
        configuration.anonymousAttempts,
      );
      await new Promise((resolve) => setTimeout(resolve, configuration.anonymousDelayMs));
      continue;
    }
    lastStatus = response.status;
    if (response.ok) {
      const bytes = Buffer.from(await response.arrayBuffer());
      if (bytes.length !== asset.size || sha256(bytes) !== asset.sha256) {
        throw new Error(`Gitee attachment ${asset.name} does not match the GitHub Release bytes`);
      }
      return;
    }
    if (attempt < configuration.anonymousAttempts && [403, 404, 418, 429, 500, 502, 503, 504].includes(response.status)) {
      writeRetryLog(
        "Gitee",
        "GET",
        "attachment-download",
        `HTTP-${response.status}`,
        attempt,
        configuration.anonymousAttempts,
      );
      await new Promise((resolve) => setTimeout(resolve, configuration.anonymousDelayMs));
      continue;
    }
    break;
  }
  throw new Error(`Anonymous download failed for ${asset.name} with HTTP ${lastStatus}`);
}

async function fetchAnonymousFromCandidates(urls, description, consume = (response) => response) {
  const candidates = urls.filter((candidate, index) => candidate && urls.indexOf(candidate) === index);
  let lastError;
  for (const [index, url] of candidates.entries()) {
    try {
      // The stable branch Raw endpoint is the fast path. If Gitee's WAF
      // blocks it, give the API-provided immutable blob URL a full retry budget.
      return await fetchAnonymousWithAttempts(
        url,
        description,
        index === 0 ? Math.min(3, configuration.anonymousAttempts) : configuration.anonymousAttempts,
        consume,
      );
    } catch (error) {
      lastError = error;
      if (index < candidates.length - 1) {
        writeRetryLog(
          "Gitee",
          "GET",
          "anonymous-fallback",
          "primary-failed",
          index + 1,
          candidates.length,
        );
      }
    }
  }
  throw lastError;
}

async function fetchAnonymousWithAttempts(url, description, attempts, consume) {
  let lastStatus = 0;
  for (let attempt = 1; attempt <= attempts; attempt += 1) {
    let response;
    try {
      response = await fetch(url, {
        redirect: "follow",
        headers: { "user-agent": "Mozilla/5.0 (compatible; GPTEasy release sync)" },
        signal: AbortSignal.timeout(configuration.requestTimeoutMs),
      });
    } catch (error) {
      if (attempt >= attempts) throw error;
      writeRetryLog("Gitee", "GET", "anonymous-download", "network-error", attempt, attempts);
      await new Promise((resolve) => setTimeout(resolve, configuration.anonymousDelayMs));
      continue;
    }
    lastStatus = response.status;
    if (response.ok) {
      try {
        return await consume(response);
      } catch (error) {
        if (attempt >= attempts) throw error;
        writeRetryLog("Gitee", "GET", "anonymous-download", "content-error", attempt, attempts);
        await new Promise((resolve) => setTimeout(resolve, configuration.anonymousDelayMs));
        continue;
      }
    }
    if (attempt < attempts && isRetryableAnonymousStatus(response.status)) {
      writeRetryLog(
        "Gitee",
        "GET",
        "anonymous-download",
        `HTTP-${response.status}`,
        attempt,
        attempts,
      );
      await new Promise((resolve) => setTimeout(resolve, configuration.anonymousDelayMs));
      continue;
    }
    break;
  }
  throw new Error(`${description} failed with HTTP ${lastStatus}`);
}

function branchRawUrl(filePath) {
  const repository = configuration.giteeRepository
    .split("/")
    .map((segment) => encodeURIComponent(segment))
    .join("/");
  const path = filePath.split("/").map((segment) => encodeURIComponent(segment)).join("/");
  return `${configuration.giteeRawBase}/${repository}/raw/${encodeURIComponent(configuration.giteeBranch)}/${path}`;
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
    .replaceAll(configuration.giteeToken, "<REDACTED>")
    .replaceAll(configuration.githubToken, "<REDACTED>");
}
