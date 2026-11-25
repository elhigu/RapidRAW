"""Time one 504x504 denoise tile with the original model and with its stride-1 ConvTranspose
layers rewritten as Conv (same rewrite as RapidRAW's ai/onnx_rewrite.rs), on the CPU with and
without ONNX Runtime's NCHWc optimization, and with CoreML where available."""
import os, platform, subprocess, sys, time
import numpy as np, onnx, onnxruntime as ort
from onnx import numpy_helper, helper

def rewrite(src, dst):
    m = onnx.load(src)
    inits = {i.name: i for i in m.graph.initializer}
    for i, n in enumerate(m.graph.node):
        if n.op_type != "ConvTranspose":
            continue
        a = {x.name: helper.get_attribute_value(x) for x in n.attribute}
        if a.get("strides", [1, 1]) != [1, 1] or a.get("dilations", [1, 1]) != [1, 1] or a.get("group", 1) != 1 \
                or any(a.get("output_padding", [0, 0])) or "output_shape" in a:
            continue
        pads = a.get("pads", [0, 0, 0, 0])
        w = numpy_helper.to_array(inits[n.input[1]])
        kh, kw = w.shape[2:]
        wc = np.ascontiguousarray(w.transpose(1, 0, 2, 3)[:, :, ::-1, ::-1])
        name = n.input[1] + "_as_conv"
        m.graph.initializer.append(numpy_helper.from_array(wc, name))
        conv = helper.make_node("Conv", [n.input[0], name] + list(n.input[2:]), list(n.output), name=n.name + "_conv",
                                kernel_shape=[kh, kw], strides=[1, 1],
                                pads=[kh - 1 - pads[0], kw - 1 - pads[1], kh - 1 - pads[2], kw - 1 - pads[3]])
        m.graph.node.remove(n)
        m.graph.node.insert(i, conv)
    used = {x for n in m.graph.node for x in n.input}
    for init in list(m.graph.initializer):
        if init.name not in used:
            m.graph.initializer.remove(init)
    onnx.save(m, dst)

def cpu_name():
    try:
        if sys.platform == "darwin":
            return subprocess.check_output(["sysctl", "-n", "machdep.cpu.brand_string"], text=True).strip()
        if sys.platform == "win32":
            return subprocess.check_output(["powershell", "-NoProfile", "(Get-CimInstance Win32_Processor).Name"], text=True).strip()
        for line in open("/proc/cpuinfo"):
            if line.startswith("model name"):
                return line.split(":", 1)[1].strip()
        out = subprocess.check_output(["lscpu"], text=True)
        return next((l.split(":", 1)[1].strip() for l in out.splitlines() if l.startswith("Model name")), platform.processor())
    except Exception:
        return platform.processor()

x = np.random.default_rng(0).random((1, 3, 504, 504), dtype=np.float32)

def run(path, providers, level=ort.GraphOptimizationLevel.ORT_ENABLE_ALL, runs=7):
    so = ort.SessionOptions()
    so.graph_optimization_level = level
    s = ort.InferenceSession(path, so, providers=providers)
    n = s.get_inputs()[0].name
    out = s.run(None, {n: x})[0]
    t = []
    for _ in range(runs):
        a = time.perf_counter(); s.run(None, {n: x}); t.append(time.perf_counter() - a)
    return float(np.median(t)), out

rewrite("original.onnx", "rewritten.onnx")
L = ort.GraphOptimizationLevel
cases = [("CPU", ["CPUExecutionProvider"], L.ORT_ENABLE_ALL), ("CPU, NCHWc off", ["CPUExecutionProvider"], L.ORT_ENABLE_EXTENDED)]
if "CoreMLExecutionProvider" in ort.get_available_providers():
    cases.append(("CoreML", ["CoreMLExecutionProvider", "CPUExecutionProvider"], L.ORT_ENABLE_ALL))
label = os.environ.get("LABEL", platform.platform())
lines = [f"### {label}: {cpu_name()}, {os.cpu_count()} threads, ONNX Runtime {ort.__version__}", "",
         "| Runs on | Original | Rewritten | Change | Max diff |", "|---|---|---|---|---|"]
for name, providers, level in cases:
    try:
        # alternate the order so that warm-up or throttling doesn't favour one model
        to1, oo = run("original.onnx", providers, level)
        tr1, orr = run("rewritten.onnx", providers, level)
        tr2, _ = run("rewritten.onnx", providers, level)
        to2, _ = run("original.onnx", providers, level)
        to, tr = min(to1, to2), min(tr1, tr2)
        lines.append(f"| {name} | {to:.3f} s | {tr:.3f} s | {(tr / to - 1) * 100:+.0f}% | {np.abs(oo - orr).max():.1e} |")
    except Exception as e:
        lines.append(f"| {name} | failed: {str(e).splitlines()[0][:120]} | | | |")
text = "\n".join(lines) + "\n"
print(text)
if os.environ.get("GITHUB_STEP_SUMMARY"):
    open(os.environ["GITHUB_STEP_SUMMARY"], "a").write(text + "\n")
