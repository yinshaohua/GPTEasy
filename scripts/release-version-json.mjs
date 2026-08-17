import { readFile, writeFile } from "node:fs/promises";
import path from "node:path";

const [mode, repositoryRoot, version] = process.argv.slice(2);
if (!mode || !repositoryRoot || !["read", "write"].includes(mode)) {
  throw new Error("usage: release-version-json.mjs <read|write> <repository-root> [version]");
}

const files = {
  package: path.join(repositoryRoot, "package.json"),
  packageLock: path.join(repositoryRoot, "package-lock.json"),
  tauri: path.join(repositoryRoot, "src-tauri", "tauri.conf.json"),
};
const packageJson = JSON.parse(await readFile(files.package, "utf8"));
const packageLock = JSON.parse(await readFile(files.packageLock, "utf8"));
const tauri = JSON.parse(await readFile(files.tauri, "utf8"));

if (!packageLock.packages?.[""]) {
  throw new Error("package-lock.json does not contain the root package entry");
}

if (mode === "write") {
  if (!version) {
    throw new Error("write mode requires a version");
  }
  packageJson.version = version;
  packageLock.version = version;
  packageLock.packages[""].version = version;
  tauri.version = version;
  await Promise.all([
    writeFile(files.package, `${JSON.stringify(packageJson, null, 2)}\n`, "utf8"),
    writeFile(files.packageLock, `${JSON.stringify(packageLock, null, 2)}\n`, "utf8"),
    writeFile(files.tauri, `${JSON.stringify(tauri, null, 2)}\n`, "utf8"),
  ]);
}

process.stdout.write(
  JSON.stringify({
    package: packageJson.version,
    packageLock: packageLock.version,
    packageLockRoot: packageLock.packages[""].version,
    tauri: tauri.version,
  }),
);
