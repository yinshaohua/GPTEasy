/* global Buffer, URL, process */

import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import http from "node:http";
import { test } from "node:test";

const TAG = "v1.2.3";
const INSTALLER = "GPTEasy_1.2.3_x64-setup.exe";
const SIGNATURE = `${INSTALLER}.sig`;
const INSTALLER_BYTES = Buffer.from("accepted-windows-installer");
const SIGNATURE_TEXT = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIG1pbmlzaWduIHNlY3JldCBrZXkKUlVRZjZMUkNHQTlpNTU5cjNnN1YxcU55SkRBcEdpcDhNZnFjYWRJZ1Q5Q3VoVjNFTWhIb04xbUdUa1VpZEYvejdTcmxRZ1hkeThvZmpiN2JOSkp5bERPb2NyQ284S0x6WndvPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNTU2MTkzMzM1CWZpbGU6dGVzdAp5L3JVdzJ5OC9oT1VZalpVNzFlSHAvV28xS1o0MGZHeTJWSkVEbDM0WE1KTStUWDQ4U3MvMTd1M0l2SWZiVlIxRmtaWlNOQ2lzUWJ1UVkrYkh3aEVCZz09";

test("首次同步验证所有附件后最后推进正式清单", async () => {
  const adapter = await startAdapter();
  try {
    const result = await runSync(adapter.baseUrl);
    assert.equal(result.code, 0, result.stderr);
    assert.equal(adapter.state.uploads.size, 3);
    assert.equal(adapter.state.manifests.length, 1);
    assert.ok(adapter.state.records.some((record) => record.operation === "anonymous-raw-baseline"));
    assert.equal(adapter.state.records.at(-1).operation, "manifest-write");
    const manifest = adapter.state.manifests[0];
    assert.deepEqual(Object.keys(manifest.platforms), ["windows-x86_64"]);
    assert.equal(manifest.version, "1.2.3");
    assert.equal(manifest.notes, "正式中文发布说明");
    assert.equal(manifest.pub_date, "2026-08-18T08:00:00Z");
    assert.equal(manifest.platforms["windows-x86_64"].signature, SIGNATURE_TEXT);
    assert.match(manifest.platforms["windows-x86_64"].url, /\/releases\/download\/v1\.2\.3\/GPTEasy_1\.2\.3_x64-setup\.exe$/);
    assert.ok(adapter.state.records.filter((record) => record.operation === "anonymous-download")
      .every((record) => record.authorization === undefined));
  } finally {
    await adapter.close();
  }
});

test("Release 正文中的字面量转义换行会规范化为 Markdown 换行", async () => {
  const adapter = await startAdapter({
    releasePatch: { body: "第一行\\n\\n### 更新\\n- 第二行" },
  });
  try {
    const result = await runSync(adapter.baseUrl);
    assert.equal(result.code, 0, result.stderr);
    assert.equal(adapter.state.manifests[0].notes, "第一行\n\n### 更新\n- 第二行");
    const releaseCreate = adapter.state.records.find((record) => record.operation === "release-create");
    assert.equal(new URLSearchParams(releaseCreate.body).get("body"), "第一行\n\n### 更新\n- 第二行");
  } finally {
    await adapter.close();
  }
});

test("部分上传后重跑会复用匹配附件并补传缺失附件", async () => {
  const adapter = await startAdapter({
    releaseExists: true,
    uploads: new Map([[INSTALLER, INSTALLER_BYTES]]),
  });
  try {
    const result = await runSync(adapter.baseUrl);
    assert.equal(result.code, 0, result.stderr);
    assert.deepEqual(adapter.state.uploadedThisRun.sort(), ["SHA256SUMS.txt", SIGNATURE].sort());
    assert.equal(adapter.state.manifests.length, 1);
  } finally {
    await adapter.close();
  }
});

test("已有 Release 正文变化时以表单编码更新数值 Release ID", async () => {
  const adapter = await startAdapter({
    releaseExists: true,
    releaseResponsePatch: { body: "旧说明" },
  });
  try {
    const result = await runSync(adapter.baseUrl);
    assert.equal(result.code, 0, result.stderr);
    const update = adapter.state.records.find((record) => record.operation === "release-update");
    assert.ok(update, "expected an existing Release update");
    assert.equal(update.path, "/gitee/repos/dist/releases/releases/42");
    assert.match(update.contentType, /^application\/x-www-form-urlencoded/);
    assert.equal(new URLSearchParams(update.body).get("body"), "正式中文发布说明");
  } finally {
    await adapter.close();
  }
});

test("Gitee 附件上传遇到瞬时 5xx 时有限重试后再推进清单", async () => {
  const adapter = await startAdapter({
    transientUploadFailures: new Map([[INSTALLER, 2]]),
  });
  try {
    const result = await runSync(adapter.baseUrl);
    assert.equal(result.code, 0, result.stderr);
    assert.equal(adapter.state.uploadAttempts.get(INSTALLER), 3);
    assert.equal(adapter.state.uploads.get(INSTALLER)?.equals(INSTALLER_BYTES), true);
    assert.equal(adapter.state.manifests.length, 1);
    assert.equal(adapter.state.records.at(-1).operation, "manifest-write");
  } finally {
    await adapter.close();
  }
});

test("Gitee 附件上传遇到 429 时有限重试后再推进清单", async () => {
  const adapter = await startAdapter({
    transientUploadFailures: new Map([[INSTALLER, 1]]),
    transientUploadStatus: 429,
  });
  try {
    const result = await runSync(adapter.baseUrl);
    assert.equal(result.code, 0, result.stderr);
    assert.equal(adapter.state.uploadAttempts.get(INSTALLER), 2);
    assert.equal(adapter.state.manifests.length, 1);
  } finally {
    await adapter.close();
  }
});

test("Gitee Release 查询遇到瞬时 503 时有限重试", async () => {
  const adapter = await startAdapter({ transientGiteeReleaseFailures: 2 });
  try {
    const result = await runSync(adapter.baseUrl);
    assert.equal(result.code, 0, result.stderr);
    assert.equal(adapter.state.giteeReleaseAttempts, 3);
    assert.equal(adapter.state.manifests.length, 1);
  } finally {
    await adapter.close();
  }
});

test("GitHub Release 附件下载遇到瞬时 503 时有限重试", async () => {
  const adapter = await startAdapter({ transientGithubAssetFailures: 1 });
  try {
    const result = await runSync(adapter.baseUrl);
    assert.equal(result.code, 0, result.stderr);
    assert.equal(adapter.state.githubAssetAttempts.get(INSTALLER), 2);
    assert.equal(adapter.state.manifests.length, 1);
  } finally {
    await adapter.close();
  }
});

test("匿名附件连接中断后会在超时预算内重试", async () => {
  const adapter = await startAdapter({ transientAnonymousNetworkFailures: 1 });
  try {
    const result = await runSync(adapter.baseUrl);
    assert.equal(result.code, 0, result.stderr);
    assert.equal(adapter.state.anonymousNetworkFailures, 0);
    assert.equal(adapter.state.manifests.length, 1);
  } finally {
    await adapter.close();
  }
});

test("Gitee 错误响应中的认证 Token 会被脱敏", async () => {
  const adapter = await startAdapter({ failGiteeRequestWithToken: true });
  try {
    const result = await runSync(adapter.baseUrl);
    assert.notEqual(result.code, 0);
    assert.match(result.stderr, /<REDACTED>/);
    assert.doesNotMatch(result.stderr, /gitee-test-token/);
  } finally {
    await adapter.close();
  }
});

test("同名附件内容冲突时停止且不推进正式清单", async () => {
  const adapter = await startAdapter({
    releaseExists: true,
    uploads: new Map([[INSTALLER, Buffer.from("different-installer")]]),
  });
  try {
    const result = await runSync(adapter.baseUrl);
    assert.notEqual(result.code, 0);
    assert.match(result.stderr, /does not match the GitHub Release bytes/);
    assert.equal(adapter.state.manifests.length, 0);
  } finally {
    await adapter.close();
  }
});

test("匿名附件下载失败时保持旧清单不变", async () => {
  const adapter = await startAdapter({ failAnonymousName: INSTALLER });
  try {
    const result = await runSync(adapter.baseUrl);
    assert.notEqual(result.code, 0);
    assert.match(result.stderr, /Anonymous download failed/);
    assert.equal(adapter.state.manifests.length, 0);
  } finally {
    await adapter.close();
  }
});

test("旧版本人工重跑不会覆盖较新的正式清单", async () => {
  const adapter = await startAdapter({
    currentManifest: {
      version: "2.0.0",
      notes: "newer",
      pub_date: "2026-08-19T08:00:00Z",
      platforms: {
        "windows-x86_64": {
          url: "http://127.0.0.1:1/installer.exe",
          signature: SIGNATURE_TEXT,
          sha256: "a".repeat(64),
          size: 1,
        },
      },
    },
  });
  try {
    const result = await runSync(adapter.baseUrl);
    assert.notEqual(result.code, 0);
    assert.match(result.stderr, /refusing to replace newer manifest/);
    assert.equal(adapter.state.uploads.size, 0);
    assert.equal(adapter.state.manifests.length, 0);
  } finally {
    await adapter.close();
  }
});

test("草稿或预发布 GitHub Release 不进入正式分发", async () => {
  for (const releasePatch of [{ draft: true }, { prerelease: true }]) {
    const adapter = await startAdapter({ releasePatch });
    try {
      const result = await runSync(adapter.baseUrl);
      assert.notEqual(result.code, 0);
      assert.match(result.stderr, /published stable release/);
      assert.equal(adapter.state.uploads.size, 0);
      assert.equal(adapter.state.manifests.length, 0);
    } finally {
      await adapter.close();
    }
  }
});

test("缺少 Windows x64 NSIS 产物时不创建正式分发", async () => {
  const adapter = await startAdapter({
    releaseAssets: [
      { name: "GPTEasy_1.2.3_arm64-setup.exe" },
      { name: "GPTEasy_1.2.3_arm64-setup.exe.sig" },
    ],
  });
  try {
    const result = await runSync(adapter.baseUrl);
    assert.notEqual(result.code, 0);
    assert.match(result.stderr, /must include GPTEasy_1\.2\.3_x64-setup\.exe/);
    assert.equal(adapter.state.uploads.size, 0);
    assert.equal(adapter.state.manifests.length, 0);
  } finally {
    await adapter.close();
  }
});

test("安装包文件名版本与 Release Tag 不同时不进入正式分发", async () => {
  const adapter = await startAdapter({
    releaseAssets: [
      { name: "GPTEasy_1.1.0_x64-setup.exe" },
      { name: "GPTEasy_1.1.0_x64-setup.exe.sig" },
    ],
  });
  try {
    const result = await runSync(adapter.baseUrl);
    assert.notEqual(result.code, 0);
    assert.match(result.stderr, /must include GPTEasy_1\.2\.3_x64-setup\.exe/);
    assert.equal(adapter.state.uploads.size, 0);
    assert.equal(adapter.state.manifests.length, 0);
  } finally {
    await adapter.close();
  }
});

test("首次发布的 Raw 匿名读取不可用时不推进分发", async () => {
  const adapter = await startAdapter({ failRawBaseline: true });
  try {
    const result = await runSync(adapter.baseUrl);
    assert.notEqual(result.code, 0);
    assert.match(result.stderr, /Raw baseline download failed/);
    assert.equal(adapter.state.uploads.size, 0);
    assert.equal(adapter.state.manifests.length, 0);
  } finally {
    await adapter.close();
  }
});

test("固定分支 Raw 被 418 拦截时会回退到 API blob 并继续同步", async () => {
  const adapter = await startAdapter({
    currentManifest: {
      version: "1.1.1",
      notes: "previous",
      pub_date: "2026-08-18T08:00:00Z",
      platforms: {
        "windows-x86_64": {
          url: "http://127.0.0.1:1/installer.exe",
          signature: SIGNATURE_TEXT,
          sha256: "a".repeat(64),
          size: 1,
        },
      },
    },
    failRawManifestBranch: true,
    transientRawManifestFailures: 2,
  });
  try {
    const result = await runSync(adapter.baseUrl);
    assert.equal(result.code, 0, result.stderr);
    assert.equal(adapter.state.rawManifestAttempts, 3);
    assert.equal(adapter.state.rawManifestBranchAttempts, 3);
    assert.equal(adapter.state.manifests.length, 1);
  } finally {
    await adapter.close();
  }
});

test("Gitee API 提供内嵌清单时不依赖匿名 Raw", async () => {
  const adapter = await startAdapter({
    currentManifest: {
      version: "1.1.1",
      notes: "previous",
      pub_date: "2026-08-18T08:00:00Z",
      platforms: {
        "windows-x86_64": {
          url: "http://127.0.0.1:1/installer.exe",
          signature: SIGNATURE_TEXT,
          sha256: "a".repeat(64),
          size: 1,
        },
      },
    },
    embeddedManifest: true,
    failRawManifestBranch: true,
  });
  try {
    const result = await runSync(adapter.baseUrl);
    assert.equal(result.code, 0, result.stderr);
    assert.equal(adapter.state.rawManifestAttempts, 0);
    assert.equal(adapter.state.rawManifestBranchAttempts, 0);
    assert.equal(adapter.state.manifests.length, 1);
  } finally {
    await adapter.close();
  }
});

async function runSync(baseUrl) {
  const child = spawn(process.execPath, ["scripts/sync-gitee-release.mjs"], {
    cwd: process.cwd(),
    env: {
      ...process.env,
      GITHUB_TOKEN: "github-test-token",
      GITHUB_REPOSITORY: "source/project",
      RELEASE_TAG: TAG,
      GITEE_TOKEN: "gitee-test-token",
      GITEE_REPOSITORY: "dist/releases",
      GITEE_DEFAULT_BRANCH: "main",
      GITHUB_API_URL: `${baseUrl}/github`,
      GITEE_API_BASE_URL: `${baseUrl}/gitee`,
      GITEE_RAW_BASE_URL: baseUrl,
      GITEE_SYNC_TEST_MODE: "1",
      GITEE_SYNC_ANONYMOUS_ATTEMPTS: "3",
      GITEE_SYNC_REQUEST_ATTEMPTS: "3",
      GITEE_SYNC_RETRY_DELAY_MS: "0",
    },
    stdio: ["ignore", "pipe", "pipe"],
  });
  let stdout = "";
  let stderr = "";
  child.stdout.setEncoding("utf8").on("data", (chunk) => { stdout += chunk; });
  child.stderr.setEncoding("utf8").on("data", (chunk) => { stderr += chunk; });
  const code = await new Promise((resolve) => child.on("close", resolve));
  return { code, stdout, stderr };
}

async function startAdapter(options = {}) {
  const state = {
    releaseExists: options.releaseExists ?? false,
    uploads: options.uploads ?? new Map(),
    uploadedThisRun: [],
    uploadAttempts: new Map(),
    transientUploadFailures: options.transientUploadFailures ?? new Map(),
    transientUploadStatus: options.transientUploadStatus ?? 502,
    transientGiteeReleaseFailures: options.transientGiteeReleaseFailures ?? 0,
    transientGithubAssetFailures: options.transientGithubAssetFailures ?? 0,
    transientAnonymousNetworkFailures: options.transientAnonymousNetworkFailures ?? 0,
    anonymousNetworkFailures: options.transientAnonymousNetworkFailures ?? 0,
    giteeReleaseAttempts: 0,
    githubAssetAttempts: new Map(),
    failGiteeRequestWithToken: options.failGiteeRequestWithToken ?? false,
    failAnonymousName: options.failAnonymousName,
    currentManifest: options.currentManifest,
    releasePatch: options.releasePatch ?? {},
    releaseResponsePatch: options.releaseResponsePatch ?? {},
    releaseAssets: options.releaseAssets,
    failRawBaseline: options.failRawBaseline ?? false,
    failRawManifestBranch: options.failRawManifestBranch ?? false,
    embeddedManifest: options.embeddedManifest ?? false,
    transientRawManifestFailures: options.transientRawManifestFailures ?? 0,
    rawManifestAttempts: 0,
    rawManifestBranchAttempts: 0,
    records: [],
    manifests: [],
  };
  const server = http.createServer(async (request, response) => {
    const url = new URL(request.url, "http://127.0.0.1");
    const body = await requestBody(request);
    const authorization = request.headers.authorization;
    const record = { method: request.method, path: url.pathname, authorization, contentType: request.headers["content-type"] };

    if (url.pathname === `/github/repos/source/project/releases/tags/${TAG}`) {
      assert.equal(authorization, "Bearer github-test-token");
      const releaseAssets = state.releaseAssets ?? [
        { name: INSTALLER },
        { name: SIGNATURE },
      ];
      return json(response, 200, {
        tag_name: TAG,
        name: "GPTEasy 1.2.3",
        body: "正式中文发布说明",
        draft: false,
        prerelease: false,
        published_at: "2026-08-18T08:00:00Z",
        assets: releaseAssets.map((asset) => ({
          ...asset,
          url: `${origin(server)}/github-assets/${asset.name}`,
        })),
        ...state.releasePatch,
      });
    }
    if (url.pathname.startsWith("/github-assets/")) {
      assert.equal(authorization, "Bearer github-test-token");
      const name = decodeURIComponent(url.pathname.slice("/github-assets/".length));
      const attempts = (state.githubAssetAttempts.get(name) ?? 0) + 1;
      state.githubAssetAttempts.set(name, attempts);
      if (attempts <= state.transientGithubAssetFailures) return bytes(response, 503, Buffer.from("temporary"));
      return bytes(response, 200, name === INSTALLER ? INSTALLER_BYTES : Buffer.from(SIGNATURE_TEXT));
    }
    if (url.pathname.startsWith("/gitee/")) {
      assert.equal(authorization, "Bearer gitee-test-token");
    }
    if (request.method === "GET" && url.pathname.endsWith(`/releases/tags/${TAG}`)) {
      if (state.failGiteeRequestWithToken) return json(response, 500, { message: `token=${state.giteeToken ?? "gitee-test-token"}` });
      state.giteeReleaseAttempts += 1;
      if (state.giteeReleaseAttempts <= state.transientGiteeReleaseFailures) return json(response, 503, { message: "temporary" });
      if (!state.releaseExists) return json(response, 404, { message: "not found" });
      return json(response, 200, releaseResponse(state, server));
    }
    if (request.method === "POST" && url.pathname.endsWith("/releases")) {
      state.releaseExists = true;
      state.records.push({ ...record, body: body.toString("utf8"), operation: "release-create" });
      assert.match(record.contentType, /^application\/x-www-form-urlencoded/);
      return json(response, 201, releaseResponse(state, server));
    }
    if (request.method === "PATCH" && url.pathname.endsWith("/releases/42")) {
      assert.match(record.contentType, /^application\/x-www-form-urlencoded/);
      state.records.push({ ...record, body: body.toString("utf8"), operation: "release-update" });
      return json(response, 200, releaseResponse(state, server));
    }
    if (request.method === "GET" && url.pathname.endsWith("/contents/latest.md")) {
      if (!state.currentManifest) return json(response, 404, { message: "not found" });
      if (state.embeddedManifest) {
        const content = Buffer.from(`${JSON.stringify(state.currentManifest)}\n`).toString("base64");
        return json(response, 200, { sha: "manifest-sha", content, encoding: "base64" });
      }
      return json(response, 200, { sha: "manifest-sha", download_url: `${origin(server)}/raw/blob/latest.md` });
    }
    if (request.method === "GET" && url.pathname.endsWith("/contents/README.md")) {
      return json(response, 200, { download_url: `${origin(server)}/raw/README.md` });
    }
    if (request.method === "POST" && /\/releases\/42\/attach_files$/.test(url.pathname)) {
      assert.match(record.contentType, /^multipart\/form-data; boundary=/);
      const name = multipartFilename(body);
      const attempts = (state.uploadAttempts.get(name) ?? 0) + 1;
      state.uploadAttempts.set(name, attempts);
      if (attempts <= (state.transientUploadFailures.get(name) ?? 0)) {
        return json(response, state.transientUploadStatus, { message: "temporary upload failure" });
      }
      state.uploads.set(name, multipartFile(body));
      state.uploadedThisRun.push(name);
      state.records.push({ ...record, operation: "upload" });
      return json(response, 201, attachmentResponse(name, state, server));
    }
    if (request.method === "GET" && url.pathname.startsWith(`/releases/download/${TAG}/`)) {
      assert.equal(authorization, undefined);
      const name = decodeURIComponent(url.pathname.slice(`/releases/download/${TAG}/`.length));
      if (state.anonymousNetworkFailures > 0) {
        state.anonymousNetworkFailures -= 1;
        request.socket.destroy();
        return;
      }
      state.records.push({ ...record, operation: "anonymous-download" });
      if (state.failAnonymousName === name) return bytes(response, 403, Buffer.from("forbidden"));
      return bytes(response, state.uploads.has(name) ? 200 : 404, state.uploads.get(name) ?? Buffer.from("missing"));
    }
    if (request.method === "GET" && ["/raw/latest.md", "/raw/blob/latest.md"].includes(url.pathname)) {
      assert.equal(authorization, undefined);
      state.rawManifestAttempts += 1;
      if (state.rawManifestAttempts <= state.transientRawManifestFailures) return bytes(response, 418, Buffer.from("warming up"));
      return json(response, 200, state.currentManifest);
    }
    if (request.method === "GET" && url.pathname === "/dist/releases/raw/main/latest.md") {
      assert.equal(authorization, undefined);
      state.rawManifestBranchAttempts += 1;
      if (state.failRawManifestBranch) return bytes(response, 418, Buffer.from("blocked"));
      return json(response, 200, state.currentManifest);
    }
    if (request.method === "GET" && url.pathname === "/raw/README.md") {
      assert.equal(authorization, undefined);
      state.records.push({ ...record, operation: "anonymous-raw-baseline" });
      if (state.failRawBaseline) return bytes(response, 403, Buffer.from("forbidden"));
      return bytes(response, 200, Buffer.from("# GPTEasy Releases\n"));
    }
    if (["POST", "PUT"].includes(request.method) && url.pathname.endsWith("/contents/latest.md")) {
      assert.match(record.contentType, /^application\/x-www-form-urlencoded/);
      const payload = new URLSearchParams(body.toString("utf8"));
      state.manifests.push(JSON.parse(Buffer.from(payload.get("content"), "base64").toString("utf8")));
      state.records.push({ ...record, operation: "manifest-write" });
      return json(response, 201, {});
    }
    return json(response, 404, { message: "unhandled" });
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  return {
    baseUrl: origin(server),
    state,
    close: () => new Promise((resolve, reject) => {
      server.close((error) => error ? reject(error) : resolve());
      server.closeAllConnections();
    }),
  };
}

function releaseResponse(state, server) {
  return {
    id: 42,
    tag_name: TAG,
    name: "GPTEasy 1.2.3",
    body: "正式中文发布说明",
    attach_files: [...state.uploads.keys()].map((name) => attachmentResponse(name, state, server)),
    ...state.releaseResponsePatch,
  };
}

function attachmentResponse(name, state, server) {
  return {
    id: [...state.uploads.keys()].indexOf(name) + 1,
    name,
    browser_download_url: `${origin(server)}/releases/download/${TAG}/${encodeURIComponent(name)}`,
  };
}

function multipartFilename(body) {
  const match = body.toString("latin1").match(/filename="([^"\r\n]+)"/);
  assert.ok(match, "multipart upload must include a filename");
  return match[1];
}

function multipartFile(body) {
  const text = body.toString("latin1");
  const headerEnd = text.indexOf("\r\n\r\n");
  assert.notEqual(headerEnd, -1, "multipart upload must include a part body");
  const end = text.lastIndexOf("\r\n--");
  assert.ok(end > headerEnd, "multipart upload must include a closing boundary");
  return body.subarray(headerEnd + 4, end);
}

function origin(server) {
  return `http://127.0.0.1:${server.address().port}`;
}

function requestBody(request) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    request.on("data", (chunk) => chunks.push(chunk));
    request.on("end", () => resolve(Buffer.concat(chunks)));
    request.on("error", reject);
  });
}

function json(response, status, value) {
  const body = Buffer.from(JSON.stringify(value));
  response.writeHead(status, { "Content-Type": "application/json", "Content-Length": body.length });
  response.end(body);
}

function bytes(response, status, value) {
  response.writeHead(status, { "Content-Type": "application/octet-stream", "Content-Length": value.length });
  response.end(value);
}
