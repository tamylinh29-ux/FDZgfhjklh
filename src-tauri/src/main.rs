#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::SystemTime,
};
use tauri::{AppHandle, Emitter, State};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VmConfig {
    id: String,
    name: String,
    qemu_path: String,
    architecture: String,
    disk_path: String,
    memory_mb: u32,
    cpu_count: u32,
    firmware: String,
    graphics: String,
    network: String,
    extra_args: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct VmStatus {
    id: String,
    state: String,
    pid: Option<u32>,
    started_at: Option<u64>,
}

struct RunningVm {
    child: Child,
    config: VmConfig,
    started_at: u64,
}

#[derive(Default)]
struct ProcessState {
    running: HashMap<String, RunningVm>,
    configs: HashMap<String, VmConfig>,
}

type SharedState = Arc<Mutex<ProcessState>>;

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn validate_config(cfg: &VmConfig) -> Result<(), String> {
    if cfg.id.trim().is_empty() || cfg.name.trim().is_empty() {
        return Err("VM identifier and name are required.".into());
    }
    if cfg.memory_mb < 256 || cfg.memory_mb > 1_048_576 {
        return Err("RAM must be between 256 MB and 1,048,576 MB.".into());
    }
    if cfg.cpu_count == 0 || cfg.cpu_count > 256 {
        return Err("CPU count must be between 1 and 256.".into());
    }
    if !Path::new(&cfg.disk_path).is_file() {
        return Err(format!("Disk image not found: {}", cfg.disk_path));
    }
    if !Path::new(&cfg.qemu_path).is_file() {
        return Err(format!("QEMU executable not found: {}", cfg.qemu_path));
    }
    let valid_arch = ["x86_64", "aarch64", "riscv64"];
    if !valid_arch.contains(&cfg.architecture.as_str()) {
        return Err("Architecture must be x86_64, aarch64, or riscv64.".into());
    }
    Ok(())
}

fn qemu_arguments(cfg: &VmConfig) -> Vec<String> {
    let mut args = vec![
        "-name".into(), cfg.name.clone(),
        "-m".into(), cfg.memory_mb.to_string(),
        "-smp".into(), cfg.cpu_count.to_string(),
        "-drive".into(), format!("file={},format=qcow2,if=virtio", cfg.disk_path),
    ];

    if cfg.firmware == "uefi" {
        // QEMU firmware is platform-specific and must be installed separately.
        args.extend(["-bios".into(), "edk2-aarch64-code.fd".into()]);
    }
    match cfg.graphics.as_str() {
        "virtio" => args.extend(["-device".into(), "virtio-vga".into()]),
        "none" => args.push("-nographic".into()),
        _ => args.extend(["-vga".into(), "std".into()]),
    }
    match cfg.network.as_str() {
        "none" => args.push("-nic".into(), "none".into()),
        "user" => args.push("-nic".into(), "user,model=virtio-net-pci".into()),
        _ => args.push("-nic".into(), "user,model=e1000".into()),
    }
    // Extra arguments are intentionally tokenized by whitespace. Avoid putting
    // secrets or shell syntax here; arguments are passed directly to QEMU.
    args.extend(cfg.extra_args.split_whitespace().map(str::to_owned));
    args
}

fn emit_log(app: &AppHandle, vm_id: &str, stream: &str, line: String) {
    let lower = line.to_lowercase();
    let mut category = "";
    let mut suggestion = "";
    if lower.contains("kvm") || lower.contains("whpx") || lower.contains("acceleration") {
        category = "virtualization";
        suggestion = "Host acceleration is unavailable. Check that CPU virtualization is enabled and use a supported QEMU accelerator.";
    } else if lower.contains("out of memory") || lower.contains("cannot allocate memory") {
        category = "memory";
        suggestion = "The guest could not allocate memory. Reduce the VM RAM allocation and close other applications.";
    } else if lower.contains("invalid accelerator") || lower.contains("unsupported machine") {
        category = "architecture";
        suggestion = "The selected machine or accelerator may not match the guest architecture. Review the architecture preset.";
    } else if lower.contains("no bootable device") || lower.contains("could not boot") || lower.contains("boot failed") {
        category = "boot";
        suggestion = "No bootable device was detected. Verify the image file and boot order, and confirm that the image matches the selected firmware.";
    } else if lower.contains("kernel panic") || lower.contains("bugcheck") {
        category = "guest-crash";
        suggestion = "The guest operating system reported a crash. Review the guest image and virtual hardware compatibility.";
    }
    let _ = app.emit("qemu-log", serde_json::json!({
        "vmId": vm_id,
        "stream": stream,
        "line": line,
        "category": category,
        "suggestion": suggestion,
        "timestamp": now_epoch()
    }));
}

fn spawn_log_reader<R: std::io::Read + Send + 'static>(
    app: AppHandle, vm_id: String, stream: &'static str, reader: R
) {
    thread::spawn(move || {
        let buffered = BufReader::new(reader);
        for line in buffered.lines() {
            match line {
                Ok(text) => emit_log(&app, &vm_id, stream, text),
                Err(error) => {
                    emit_log(&app, &vm_id, stream, format!("Log reader error: {error}"));
                    break;
                }
            }
        }
    });
}

#[tauri::command]
fn save_vm(config: VmConfig, state: State<'_, SharedState>) -> Result<(), String> {
    validate_config(&config)?;
    let mut guard = state.lock().map_err(|_| "Process state lock poisoned")?;
    guard.configs.insert(config.id.clone(), config);
    Ok(())
}

#[tauri::command]
fn start_vm(config: VmConfig, app: AppHandle, state: State<'_, SharedState>) -> Result<VmStatus, String> {
    validate_config(&config)?;
    let mut guard = state.lock().map_err(|_| "Process state lock poisoned")?;
    if guard.running.contains_key(&config.id) {
        return Err("This VM is already running.".into());
    }

    let mut command = Command::new(&config.qemu_path);
    command.args(qemu_arguments(&config))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(windows)]
    command.creation_flags(0x08000000);

    let mut child = command.spawn().map_err(|e| format!("Could not start QEMU: {e}"))?;
    let pid = child.id();
    if let Some(stdout) = child.stdout.take() {
        spawn_log_reader(app.clone(), config.id.clone(), "stdout", stdout);
    }
    if let Some(stderr) = child.stderr.take() {
        spawn_log_reader(app.clone(), config.id.clone(), "stderr", stderr);
    }

    let started_at = now_epoch();
    let id = config.id.clone();
    guard.configs.insert(id.clone(), config.clone());
    guard.running.insert(id.clone(), RunningVm { child, config, started_at });
    Ok(VmStatus { id, state: "running".into(), pid: Some(pid), started_at: Some(started_at) })
}

#[tauri::command]
fn stop_vm(id: String, state: State<'_, SharedState>) -> Result<(), String> {
    let mut guard = state.lock().map_err(|_| "Process state lock poisoned")?;
    if let Some(mut vm) = guard.running.remove(&id) {
        vm.child.kill().map_err(|e| format!("Could not terminate QEMU: {e}"))?;
        let _ = vm.child.wait();
    }
    Ok(())
}

#[tauri::command]
fn restart_vm(id: String, app: AppHandle, state: State<'_, SharedState>) -> Result<VmStatus, String> {
    let config = {
        let mut guard = state.lock().map_err(|_| "Process state lock poisoned")?;
        if let Some(mut vm) = guard.running.remove(&id) {
            let _ = vm.child.kill();
            let _ = vm.child.wait();
        }
        guard.configs.get(&id).cloned().ok_or("VM configuration not found")?
    };
    start_vm(config, app, state)
}

#[tauri::command]
fn pause_vm(id: String, state: State<'_, SharedState>) -> Result<(), String> {
    let guard = state.lock().map_err(|_| "Process state lock poisoned")?;
    let vm = guard.running.get(&id).ok_or("VM is not running")?;
    platform_signal(vm.child.id(), true)
}

#[tauri::command]
fn resume_vm(id: String, state: State<'_, SharedState>) -> Result<(), String> {
    let guard = state.lock().map_err(|_| "Process state lock poisoned")?;
    let vm = guard.running.get(&id).ok_or("VM is not running")?;
    platform_signal(vm.child.id(), false)
}

#[cfg(unix)]
fn platform_signal(pid: u32, pause: bool) -> Result<(), String> {
    let signal = if pause { "-STOP" } else { "-CONT" };
    let status = Command::new("kill").args([signal, &pid.to_string()]).status()
        .map_err(|e| format!("Could not signal process: {e}"))?;
    if status.success() { Ok(()) } else { Err("Operating system rejected process signal.".into()) }
}

#[cfg(windows)]
fn platform_signal(pid: u32, pause: bool) -> Result<(), String> {
    // PowerShell process suspension is available on supported Windows builds.
    let action = if pause { "Suspend-Process" } else { "Resume-Process" };
    let script = format!("{action} -Id {pid} -ErrorAction Stop");
    let status = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .status().map_err(|e| format!("Could not control process: {e}"))?;
    if status.success() { Ok(()) } else { Err("Process control failed; check Windows permissions.".into()) }
}

#[tauri::command]
fn list_vms(state: State<'_, SharedState>) -> Result<Vec<VmStatus>, String> {
    let mut guard = state.lock().map_err(|_| "Process state lock poisoned")?;
    let ids: Vec<String> = guard.running.keys().cloned().collect();
    let mut statuses = Vec::new();
    for id in ids {
        let mut exited = false;
        if let Some(vm) = guard.running.get_mut(&id) {
            match vm.child.try_wait() {
                Ok(Some(_)) => exited = true,
                Ok(None) => statuses.push(VmStatus {
                    id: id.clone(), state: "running".into(),
                    pid: Some(vm.child.id()), started_at: Some(vm.started_at)
                }),
                Err(error) => return Err(format!("Could not inspect VM process: {error}")),
            }
        }
        if exited { guard.running.remove(&id); }
    }
    Ok(statuses)
}

#[tauri::command]
fn detect_preset(file_path: String) -> Result<serde_json::Value, String> {
    let path = PathBuf::from(&file_path);
    if !path.is_file() { return Err("Selected image file does not exist.".into()); }
    let filename = path.file_name().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
    let text = format!("{filename} {ext}");
    let (arch, ram, cpus, graphics, firmware) = if text.contains("arm64") || text.contains("aarch64") {
        ("aarch64", 4096, 4, "virtio", "uefi")
    } else if text.contains("riscv") {
        ("riscv64", 2048, 2, "virtio", "uefi")
    } else if text.contains("windows") || text.contains("win") {
        ("x86_64", 4096, 4, "std", "bios")
    } else if text.contains("android") {
        ("x86_64", 3072, 4, "virtio", "uefi")
    } else {
        ("x86_64", 2048, 2, "virtio", "bios")
    };
    let disk_format = match ext.as_str() {
        "qcow2" => "qcow2",
        "vmdk" => "vmdk",
        "vdi" => "vdi",
        "img" => "raw",
        "iso" => "raw",
        _ => "unknown"
    };
    Ok(serde_json::json!({
        "architecture": arch, "memoryMb": ram, "cpuCount": cpus,
        "graphics": graphics, "firmware": firmware, "diskFormat": disk_format,
        "note": "Filename-based heuristic only. Confirm guest requirements and disk format before booting."
    }))
}

#[tauri::command]
fn get_qemu_version(qemu_path: String) -> Result<String, String> {
    let output = Command::new(qemu_path).arg("--version").output()
        .map_err(|e| format!("Unable to execute QEMU: {e}"))?;
    if !output.status.success() { return Err("QEMU version command failed.".into()); }
    Ok(String::from_utf8_lossy(&output.stdout).lines().next().unwrap_or("QEMU").to_string())
}

fn main() {
    let state: SharedState = Arc::new(Mutex::new(ProcessState::default()));
    tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            save_vm, start_vm, stop_vm, restart_vm, pause_vm, resume_vm,
            list_vms, detect_preset, get_qemu_version
        ])
        .run(tauri::generate_context!())
        .expect("error while running AllOS Emulator");
}
