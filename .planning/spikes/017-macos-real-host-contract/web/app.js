const invoke = window.__TAURI__.core.invoke;

function escapeHtml(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

function metric(label, value) {
  return `<article><strong>${escapeHtml(value)}</strong><span>${escapeHtml(label)}</span></article>`;
}

async function refresh() {
  const snapshot = await invoke("get_contract_snapshot");
  document.querySelector("#metrics").innerHTML = [
    metric("操作系统", `${snapshot.os}/${snapshot.arch}`),
    metric("安装范围", snapshot.current_app_scope),
    metric("Codex 应用", snapshot.app_bundles.length),
    metric("相关进程", snapshot.processes.length),
  ].join("");
  document.querySelector("#snapshot").textContent = JSON.stringify(snapshot, null, 2);
}

document.querySelector("#refresh").addEventListener("click", refresh);
document.querySelector("#write-canary").addEventListener("click", async () => {
  const value = document.querySelector("#canary").value;
  const path = await invoke("write_update_canary", { value });
  document.querySelector("#canary-result").textContent = `已写入：${path}`;
});
document.querySelector("#read-canary").addEventListener("click", async () => {
  const value = await invoke("read_update_canary");
  document.querySelector("#canary-result").textContent =
    value === null ? "未找到 canary。" : `读取成功：${value}`;
});
document.querySelector("#export").addEventListener("click", async () => {
  const path = await invoke("export_evidence");
  document.querySelector("#export-result").textContent = `证据已导出：${path}`;
});

refresh().catch((error) => {
  document.querySelector("#snapshot").textContent = String(error);
});
