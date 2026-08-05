const invoke = window.__TAURI__.core.invoke;
let lastScan = null;

function metric(label, value) {
  return `<div class="metric"><strong>${value}</strong><span>${label}</span></div>`;
}

function escapeHtml(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

async function scan() {
  const button = document.querySelector("#scan");
  button.disabled = true;
  try {
    lastScan = await invoke("scan_processes");
    document.querySelector("#summary").innerHTML = [
      metric("桌面主进程", lastScan.counts.desktop_root),
      metric("桌面 Codex 子进程", lastScan.counts.desktop_codex_child),
      metric("本机 CLI", lastScan.counts.cli),
      metric("旧版/其它宿主", lastScan.counts.legacy_or_other_host),
    ].join("");
    const cards = lastScan.processes.map((process) => `
      <article class="card">
        <strong>${escapeHtml(process.name)}</strong>
        <span class="badge">${escapeHtml(process.role)}</span>
        <div class="meta">PID ${process.pid} · PPID ${process.parent_pid ?? "-"}</div>
        <div class="meta">${escapeHtml(process.executable ?? "路径不可读")}</div>
        <div class="meta">置信度：${escapeHtml(process.confidence)} · ${escapeHtml(process.reason)}</div>
      </article>
    `);
    document.querySelector("#processes").innerHTML =
      cards.join("") || '<article class="card">没有检测到相关进程。</article>';
    document.querySelector("#plan").textContent = "请选择切换行为以生成计划。";
  } catch (error) {
    document.querySelector("#processes").innerHTML =
      `<article class="card">扫描失败：${escapeHtml(error)}</article>`;
  } finally {
    button.disabled = false;
  }
}

async function plan(decision) {
  if (!lastScan) await scan();
  const result = await invoke("build_restart_plan", {
    decision,
    processes: lastScan.processes,
  });
  document.querySelector("#plan").textContent = JSON.stringify(result, null, 2);
}

document.querySelector("#scan").addEventListener("click", scan);
document.querySelectorAll("[data-decision]").forEach((button) => {
  button.addEventListener("click", () => plan(button.dataset.decision));
});

window.__TAURI__.event.listen("process-scan-requested", scan);
scan();
