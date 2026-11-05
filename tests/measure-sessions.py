"""Run optimized codec benchmarks in separate processes and record Windows peak working set."""
import ctypes
import json
from pathlib import Path
import subprocess
from ctypes import wintypes

class MemoryCounters(ctypes.Structure):
    _fields_ = [("cb", wintypes.DWORD), ("PageFaultCount", wintypes.DWORD)] + [(name, ctypes.c_size_t) for name in ["PeakWorkingSetSize", "WorkingSetSize", "QuotaPeakPagedPoolUsage", "QuotaPagedPoolUsage", "QuotaPeakNonPagedPoolUsage", "QuotaNonPagedPoolUsage", "PagefileUsage", "PeakPagefileUsage"]]

get_memory = ctypes.windll.psapi.GetProcessMemoryInfo
get_memory.argtypes = [wintypes.HANDLE, ctypes.POINTER(MemoryCounters), wintypes.DWORD]
results = []
for count in (100_000, 150_000, 200_000):
    for level in (3, 6, 9):
        output = f"target/benchmark-{count}.icas"
        process = subprocess.Popen(["target/release/examples/session_benchmark.exe", str(count), str(level), output], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        peak = 0
        while process.poll() is None:
            info = MemoryCounters(); info.cb = ctypes.sizeof(info)
            if get_memory(int(process._handle), ctypes.byref(info), info.cb): peak = max(peak, info.PeakWorkingSetSize)
            try: process.wait(timeout=0.05)
            except subprocess.TimeoutExpired: pass
        stdout, stderr = process.communicate()
        if process.returncode: raise RuntimeError(stderr)
        result = json.loads(stdout); result["peakWorkingSetMiB"] = round(peak / 1024**2, 1)
        results.append(result)
        print(json.dumps(result), flush=True)
Path("target/session-benchmarks.json").write_text(json.dumps(results, indent=2))
