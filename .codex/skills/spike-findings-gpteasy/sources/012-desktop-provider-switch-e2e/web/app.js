const invoke = window.__TAURI__.core.invoke;

function escapeHtml(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

function metric(label, value) {
  return `<article><strong>${escapeHtml(value ?? "-")}</strong><span>${escapeHtml(label)}</span></article>`;
}

async function scanProcesses() {
  const result = await invoke("scan_processes");
  const counts = result.counts;
  document.querySelector("#process-metrics").innerHTML = [
    metric("桌面宿主", counts.desktop_root ?? 0),
    metric("bundled Codex", counts.desktop_codex_child ?? 0),
    metric("本机 CLI", counts.cli ?? 0),
  ].join("");
  document.querySelector("#processes").innerHTML =
    result.processes.map((process) => `
      <article>
        <strong>${escapeHtml(process.name)}</strong>
        <span>${escapeHtml(process.role)} · PID ${process.pid} · PPID ${process.parent_pid ?? "-"}</span>
        <small>${escapeHtml(process.relaunch ?? "不自动重启")}</small>
      </article>
    `).join("") || "没有检测到相关进程。";
}

function renderPipeline(report) {
  document.querySelectorAll("#pipeline article").forEach((node) => {
    node.classList.remove("done", "failed");
  });
  if (report.validation.ok) {
    document.querySelector('[data-stage="validation"]').classList.add("done");
  }
  for (const event of report.events) {
    const node = document.querySelector(`[data-stage="${event.category}"]`);
    if (node) node.classList.add("done");
  }
  if (report.effective) {
    document.querySelector('[data-stage="effective"]').classList.add("done");
  }
  if (report.snapshot.phase === "needs_attention") {
    document.querySelector('[data-stage="config_replaced"]').classList.add("failed");
  }
}

function renderReport(report) {
  renderPipeline(report);
  document.querySelector("#status-cards").innerHTML = [
    metric("当前供应商", report.snapshot.current_provider),
    metric("Saga 阶段", report.snapshot.phase),
    metric("有效模型", report.effective?.model),
    metric("协调状态", report.reconciliation?.state),
  ].join("");
  document.querySelector("#validation-stages").innerHTML =
    report.validation.stages.map((stage) => `
      <article class="validation-stage ${stage.ok ? "ok" : "bad"}">
        <strong>${escapeHtml(stage.name)}</strong>
        <span>${stage.duration_ms} ms</span>
      </article>
    `).join("");
  document.querySelector("#report").textContent = JSON.stringify(report, null, 2);
}

async function runDemo() {
  const button = document.querySelector("#run");
  button.disabled = true;
  button.textContent = "运行中……";
  try {
    const report = await invoke("run_demo", {
      decision: document.querySelector("#decision").value,
      providerSource: document.querySelector("#provider-source").value,
      injection: document.querySelector("#injection").value,
    });
    renderReport(report);
  } catch (error) {
    document.querySelector("#report").textContent = `运行失败：${error}`;
  } finally {
    button.disabled = false;
    button.textContent = "运行端到端流程";
  }
}

document.querySelector("#scan").addEventListener("click", scanProcesses);
document.querySelector("#run").addEventListener("click", runDemo);
document.querySelector("#export").addEventListener("click", async () => {
  try {
    const result = await invoke("export_latest_report");
    document.querySelector("#report").textContent =
      `脱敏报告：${result.path}\n大小：${result.bytes} bytes`;
  } catch (error) {
    document.querySelector("#report").textContent = String(error);
  }
});

scanProcesses().catch((error) => {
  document.querySelector("#processes").textContent = String(error);
});
