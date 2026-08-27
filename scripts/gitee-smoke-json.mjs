import { readFile } from "node:fs/promises";

const [command, ...args] = process.argv.slice(2);

switch (command) {
  case "config": {
    const [configPath, key] = args;
    const config = JSON.parse(await readFile(configPath, "utf8"));
    const value = config[key];
    if (typeof value !== "string" && typeof value !== "number") {
      throw new Error(`Gitee distribution config is missing ${key}`);
    }
    process.stdout.write(String(value));
    break;
  }
  case "urlencode": {
    process.stdout.write(encodeURIComponent(args[0] ?? ""));
    break;
  }
  case "manifest": {
    const [tag, asset, sha256] = args;
    process.stdout.write(
      `${JSON.stringify({
        schemaVersion: 1,
        kind: "gitee-api-smoke",
        tag,
        asset,
        sha256,
      })}\n`,
    );
    break;
  }
  case "verify-manifest": {
    const [manifestPath, tag, sha256] = args;
    const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
    if (
      manifest.kind !== "gitee-api-smoke" ||
      manifest.tag !== tag ||
      manifest.sha256 !== sha256
    ) {
      throw new Error("anonymous Raw smoke manifest does not match");
    }
    break;
  }
  case "download-url": {
    const [metadataPath, expectedRawBase] = args;
    const metadata = JSON.parse(await readFile(metadataPath, "utf8"));
    const downloadUrl = new URL(metadata.download_url);
    const expectedOrigin = new URL(expectedRawBase).origin;
    if (
      downloadUrl.origin !== expectedOrigin ||
      !downloadUrl.pathname.includes("/blobs/")
    ) {
      throw new Error("Gitee content metadata returned an unexpected Raw URL");
    }
    process.stdout.write(downloadUrl.toString());
    break;
  }
  case "report": {
    const [tag, releaseId, attachment, manifest] = args;
    process.stdout.write(
      `${JSON.stringify({
        passed: true,
        formalManifestAdvanced: false,
        tag,
        releaseId: Number(releaseId),
        anonymousAttachment: attachment,
        anonymousRawManifest: manifest,
      })}\n`,
    );
    break;
  }
  case "api-error": {
    const response = JSON.parse(await readFile(args[0], "utf8"));
    const token = process.env.GITEE_TOKEN ?? "";
    const redact = (value) => {
      const text = String(value ?? "").slice(0, 500);
      return token ? text.replaceAll(token, "<REDACTED>") : text;
    };
    process.stdout.write(
      `${JSON.stringify({
        errorCode: redact(response.error_code),
        errorName: redact(response.error_code_name),
        message: redact(response.error_message ?? response.message),
      })}\n`,
    );
    break;
  }
  default:
    throw new Error(`unknown Gitee smoke JSON command: ${command ?? ""}`);
}
