import { readFile } from "node:fs/promises";

const [responsePath, assetPath, testMode = "0"] = process.argv.slice(2);
if (!responsePath || !assetPath) {
  throw new Error("usage: gitcode-upload.mjs <upload-response> <asset> [test-mode]");
}

const upload = JSON.parse(await readFile(responsePath, "utf8"));
const url = new URL(upload.url);
if (
  url.protocol !== "https:" &&
  !(testMode === "1" && url.protocol === "http:" && url.hostname === "127.0.0.1")
) {
  throw new Error("GitCode returned an invalid attachment upload URL");
}
if (
  !upload.headers ||
  typeof upload.headers !== "object" ||
  Array.isArray(upload.headers) ||
  Object.values(upload.headers).some((value) => typeof value !== "string")
) {
  throw new Error("GitCode returned invalid attachment upload headers");
}

const response = await fetch(url, {
  method: "PUT",
  headers: upload.headers,
  body: await readFile(assetPath),
  redirect: "follow",
});
if (!response.ok) {
  throw new Error(`GitCode attachment upload returned ${response.status}`);
}
process.stdout.write(`${JSON.stringify({ uploaded: true, status: response.status })}\n`);
