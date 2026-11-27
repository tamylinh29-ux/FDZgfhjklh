(() => {
  "use strict";
  const $ = (id) => document.getElementById(id);
  const isTauri = !!(window.__TAURI__ && window.__TAURI__.core);
  const invoke = async (command, args = {}) => {
    if (!isTauri) throw new Error("Run this interface in the Tauri desktop application to control local QEMU.");
    return window.__TAURI__.core.invoke(command, args);
  };
  let vms = [];
  let activeId = null;
  let issues = new Map();
  const persistKey = "allos-vm-configs";

  const safeId = () => "vm-" + Date.now().toString(36) + "-" + Math.random().toString(36).slice(2, 7);
  const loadSaved = () => {
    try { return JSON.parse(localStorage.getItem(persistKey) || "[]"); }
    catch (_) { return []; }
  };
  const saveSaved = () => localStorage.setItem(persistKey, JSON.stringify(vms.map(({state, pid, ...cfg}) => cfg)));
  const escapeHtml = (value) => String(value ?? "").replace(/[&<>"']/g, c => ({"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;","'":"&#39;"}[c]));
  const current = () => vms.find(vm => vm.id === activeId) || vms[0] || null;

  function openModal() { $("modalBackdrop").classList.add("visible"); $("modalBackdrop").setAttribute("aria-hidden","false"); }
  function closeModal() { $("modalBackdrop").classList.remove("visible"); $("modalBackdrop").setAttribute("aria-hidden","true"); }
  function appendLog(message, error = false) {
    const output = $("logOutput");
    if (output.querySelector(".muted")) output.innerHTML = "";
    const line = document.createElement("p");
    line.className = "log-line" + (error ? " error" : "");
    line.textContent = `[${new Date().toLocaleTimeString()}] ${message}`;
    output.appendChild(line);
    output.scrollTop = output.scrollHeight;
  }
  function render() {
    const grid = $("vmGrid");
    grid.innerHTML = "";
    if (!vms.length) {
      grid.innerHTML = `<article class="empty-card"><div class="empty-icon">▧</div><h3>Start with a virtual machine</h3><p>Add a guest image and connect your local QEMU installation to get started.</p><button class="secondary-button" id="emptyCreate">Create your first VM</button></article>`;
      $("emptyCreate").addEventListener("click", openModal);
    } else {
      for (const vm of vms) {
        const card = document.createElement("article");
        card.className = "vm-card";
        card.innerHTML = `<div class="vm-card-top"><div class="os-icon">▧</div><span class="status ${vm.state === "running" ? "running" : ""}">${escapeHtml(vm.state || "stopped")}</span></div>
          <h3>${escapeHtml(vm.name)}</h3><div class="vm-meta"><span>${escapeHtml(vm.architecture)}</span><span>·</span><span>${vm.memoryMb} MB RAM</span><span>·</span><span>${vm.cpuCount} CPU</span></div>
          <div class="vm-actions"><button data-vm="${escapeHtml(vm.id)}" data-action="start">▶ Start</button><button data-vm="${escapeHtml(vm.id)}" data-action="pause">Ⅱ Pause</button><button data-vm="${escapeHtml(vm.id)}" data-action="stop">■ Stop</button></div>`;
        card.addEventListener("click", () => { activeId = vm.id; updateIsland(); });
        grid.appendChild(card);
      }
    }
    updateIsland();
  }
  function updateIsland() {
    const vm = current();
    $("islandLabel").textContent = vm ? `${vm.name} · ${vm.state || "stopped"}` : "No VM running";
    $("globalDot").classList.toggle("active", !!vm && vm.state === "running");
    $("cpuMetric").textContent = vm ? `${vm.cpuCount} vCPU` : "—";
    $("ramMetric").textContent = vm ? `${vm.memoryMb} MB` : "—";
  }
  function getFormConfig() {
    return {
      id: safeId(), name: $("vmName").value.trim(),
      qemuPath: $("qemuPath").value.trim(), architecture: $("architecture").value,
      diskPath: $("diskPath").value.trim(), memoryMb: Number($("memoryMb").value),
      cpuCount: Number($("cpuCount").value), firmware: $("firmware").value,
      graphics: $("graphics").value, network: $("network").value,
      extraArgs: $("extraArgs").value.trim()
    };
  }
  async function addVm(event) {
    event.preventDefault();
    const config = getFormConfig();
    if (!config.name || !config.qemuPath || !config.diskPath) return;
    try {
      await invoke("save_vm", {config});
      vms.push({...config, state:"stopped"});
      activeId = config.id;
      saveSaved(); render(); closeModal();
      appendLog(`Saved VM configuration: ${config.name}`);
    } catch (error) {
      alert(String(error));
    }
  }
  async function action(actionName, id = (current() || {}).id) {
    if (!id) return;
    const vm = vms.find(item => item.id === id);
    if (!vm) return;
    try {
      if (actionName === "start") {
        await invoke("start_vm", {config: vm});
        vm.state = "running";
      } else if (actionName === "stop") {
        await invoke("stop_vm", {id}); vm.state = "stopped";
      } else if (actionName === "pause") {
        await invoke("pause_vm", {id}); vm.state = "paused";
      } else if (actionName === "resume") {
        await invoke("resume_vm", {id}); vm.state = "running";
      } else if (actionName === "restart") {
        await invoke("restart_vm", {id}); vm.state = "running";
      } else if (actionName === "screenshot") {
        appendLog("Screenshot capture is not available in this build; configure a QEMU display/VNC capture workflow.");
        return;
      }
      activeId = id; render(); saveSaved(); appendLog(`${actionName} requested for ${vm.name}`);
    } catch (error) { appendLog(String(error), true); alert(String(error)); }
  }
  function showIssue(data) {
    if (!data.category) return;
    issues.set(data.vmId, data);
    $("debuggerContent").innerHTML = `<div class="debugger-ok" style="background:#fff0dc;color:#b47a2c">!</div><b>${escapeHtml(data.category.replace("-", " "))} detected</b><p>${escapeHtml(data.suggestion || data.line)}</p><button class="fix-button" id="oneTapFix">Review suggested adjustment ↗</button><p class="muted">No configuration will be changed without your confirmation.</p>`;
    $("oneTapFix").addEventListener("click", () => applySafeFix(data));
  }
  async function applySafeFix(issue) {
    const vm = vms.find(item => item.id === issue.vmId);
    if (!vm) return;
    if (issue.category === "memory") {
      const proposed = Math.max(512, Math.floor(vm.memoryMb / 2 / 256) * 256);
      if (confirm(`Reduce VM RAM from ${vm.memoryMb} MB to ${proposed} MB and restart?`)) {
        vm.memoryMb = proposed; await saveAndRestart(vm);
      }
    } else if (issue.category === "virtualization") {
      alert("Enable hardware virtualization in your computer firmware and install the matching host accelerator. This application cannot safely change host firmware settings.");
    } else if (issue.category === "architecture") {
      alert("Check that the guest image, QEMU system binary and selected architecture match. Select the correct QEMU executable and edit the VM configuration.");
    } else if (issue.category === "boot") {
      alert("Check that the image is bootable and that the selected BIOS/UEFI firmware matches the image.");
    } else {
      alert("Review the guest OS crash log and verify guest image compatibility before restarting.");
    }
  }
  async function saveAndRestart(vm) {
    try {
      await invoke("stop_vm", {id:vm.id}).catch(() => {});
      await invoke("save_vm", {config:vm});
      await invoke("start_vm", {config:vm});
      vm.state = "running"; render(); saveSaved();
    } catch (error) { appendLog(String(error), true); }
  }
  async function autoPreset() {
    const path = $("diskPath").value.trim();
    if (!path || !isTauri) return;
    try {
      const preset = await invoke("detect_preset", {filePath:path});
      $("architecture").value = preset.architecture;
      $("memoryMb").value = preset.memoryMb;
      $("cpuCount").value = preset.cpuCount;
      $("graphics").value = preset.graphics;
      $("firmware").value = preset.firmware;
      $("presetNote").textContent = `Suggested ${preset.architecture} · ${preset.memoryMb} MB RAM · ${preset.firmware.toUpperCase()} · detected format: ${preset.diskFormat}. ${preset.note}`;
    } catch (error) { $("presetNote").textContent = String(error); }
  }
  async function refreshStatuses() {
    if (!isTauri) return;
    try {
      const statuses = await invoke("list_vms");
      for (const vm of vms) {
        const status = statuses.find(item => item.id === vm.id);
        vm.state = status ? "running" : (vm.state === "paused" ? "paused" : "stopped");
      }
      render();
    } catch (error) { appendLog(String(error), true); }
  }

  $("newVmButton").addEventListener("click", openModal);
  $("emptyCreate")?.addEventListener("click", openModal);
  $("closeModal").addEventListener("click", closeModal);
  $("cancelModal").addEventListener("click", closeModal);
  $("modalBackdrop").addEventListener("click", e => { if (e.target === $("modalBackdrop")) closeModal(); });
  $("vmForm").addEventListener("submit", addVm);
  $("diskPath").addEventListener("change", autoPreset);
  $("diskPath").addEventListener("blur", autoPreset);
  $("islandToggle").addEventListener("click", () => {
    const island = $("dynamicIsland");
    const opened = island.classList.toggle("open");
    $("islandToggle").setAttribute("aria-expanded", String(opened));
  });
  $("themeToggle").addEventListener("click", () => {
    document.body.classList.toggle("dark");
    localStorage.setItem("allos-theme", document.body.classList.contains("dark") ? "dark" : "light");
  });
  document.querySelectorAll("[data-action]").forEach(button => button.addEventListener("click", () => action(button.dataset.action, button.dataset.vm || (current() || {}).id)));
  $("vmGrid").addEventListener("click", e => {
    const button = e.target.closest("button[data-action]");
    if (button) { e.stopPropagation(); action(button.dataset.action, button.dataset.vm); }
  });
  document.querySelectorAll(".island-actions [data-action]").forEach(button => button.addEventListener("click", () => action(button.dataset.action)));

  if (localStorage.getItem("allos-theme") === "dark" ||
      (!localStorage.getItem("allos-theme") && window.matchMedia?.("(prefers-color-scheme: dark)").matches)) {
    document.body.classList.add("dark");
  }
  vms = loadSaved().map(vm => ({...vm, state:"stopped"}));
  if (vms.length) activeId = vms[0].id;
  render();

  if (isTauri && window.__TAURI__.event) {
    window.__TAURI__.event.listen("qemu-log", event => {
      const data = event.payload || {};
      appendLog(`[${data.vmId || "QEMU"}] ${data.stream || "log"}: ${data.line || ""}`,
        data.stream === "stderr" || !!data.category);
      if (data.category) showIssue(data);
    });
  } else {
    appendLog("Preview mode: launch the Tauri desktop app to control QEMU processes.");
  }
  setInterval(refreshStatuses, 2500);
})();
