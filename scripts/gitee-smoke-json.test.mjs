/* global Buffer, process */

import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { promisify } from "node:util";
import { test } from "node:test";

const execFileAsync = promisify(execFile);

test("清理时将 Gitee 的空数组内容响应视为清单不存在", async () => {
  const workspace = await mkdtemp(join(tmpdir(), "gpteasy-gitee-cleanup-"));
  const metadataPath = join(workspace, "metadata.json");
  try {
    await writeFile(metadataPath, "[]\n");
    const result = await execFileAsync(
      process.execPath,
      ["scripts/gitee-smoke-json.mjs", "cleanup-metadata", metadataPath, "smoke-123-1"],
    );
    assert.equal(result.stdout, "");
    assert.equal(result.stderr, "");
  } finally {
    await rm(workspace, { recursive: true, force: true });
  }
});

test("清理时只接受匹配 tag 的 smoke 清单", async () => {
  const workspace = await mkdtemp(join(tmpdir(), "gpteasy-gitee-cleanup-"));
  const metadataPath = join(workspace, "metadata.json");
  try {
    const manifest = Buffer.from(JSON.stringify({ kind: "gitee-api-smoke", tag: "smoke-123-1" })).toString("base64");
    await writeFile(metadataPath, `${JSON.stringify({ sha: "blob-sha", content: manifest })}\n`);
    await assert.rejects(
      execFileAsync(process.execPath, [
        "scripts/gitee-smoke-json.mjs",
        "cleanup-metadata",
        metadataPath,
        "smoke-456-1",
      ]),
      /Gitee smoke manifest does not match the cleanup tag/,
    );
  } finally {
    await rm(workspace, { recursive: true, force: true });
  }
});
