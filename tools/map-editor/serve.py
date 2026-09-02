#!/usr/bin/env python3
"""地图编辑器胶水服务（docs/superpowers/specs/2026-09-01-map-editor-design.md §6）。

四环之外的开发工具：只读 data/materials.ron、写 data/scenarios/*.ron、调用
target/release/sand-harness。所有"契约"性的解析（grid 字段、材质表）都在
harness（Rust）里，这里只搬运 JSON。

    python3 tools/map-editor/serve.py            # 127.0.0.1:8765
    ssh -L 8765:localhost:8765 sunyunbo@zhustation ; 浏览器开 http://localhost:8765/
"""
import json
import re
import subprocess
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlparse

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
BIN = ROOT / "target" / "release" / "sand-harness"
SCENARIOS = ROOT / "data" / "scenarios"
OUT = HERE / "out"
NAME_RE = re.compile(r"^[a-z0-9_]{1,40}$")
RENDER_TIMEOUT_S = 300


def harness(*args, timeout=60):
    """跑一条 sand-harness 子命令，返回 (returncode, stdout, stderr)。"""
    p = subprocess.run([str(BIN), *args], cwd=ROOT, capture_output=True, text=True, timeout=timeout)
    return p.returncode, p.stdout, p.stderr


class Handler(BaseHTTPRequestHandler):
    def _send(self, code, body, ctype="application/json; charset=utf-8"):
        if isinstance(body, (dict, list)):
            body = json.dumps(body, ensure_ascii=False).encode()
        elif isinstance(body, str):
            body = body.encode()
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        u = urlparse(self.path)
        if u.path == "/":
            return self._send(200, (HERE / "index.html").read_text(encoding="utf-8"), "text/html; charset=utf-8")
        if u.path == "/materials":
            rc, out, err = harness("materials", "--json")
            return self._send(200, out, "application/json") if rc == 0 else self._send(500, {"error": err})
        if u.path == "/scenarios":
            names = sorted(p.stem for p in SCENARIOS.glob("*.ron"))
            return self._send(200, names)
        if u.path == "/load":
            name = parse_qs(u.query).get("name", [""])[0]
            if not NAME_RE.match(name):
                return self._send(400, {"error": f"非法场景名 {name!r}"})
            path = SCENARIOS / f"{name}.ron"
            if not path.exists():
                return self._send(404, {"error": f"{path.name} 不存在"})
            rc, out, err = harness("rasterize", str(path))
            if rc != 0:
                return self._send(500, {"error": err})
            grid = json.loads(out)
            # 页面不解析 RON 的网格；seed/ticks/script 只做轻量文本提取供侧栏预填
            grid["ron"] = path.read_text(encoding="utf-8")
            return self._send(200, grid)
        if u.path.startswith("/out/"):
            fname = u.path[len("/out/"):]
            if not re.match(r"^[a-z0-9_]{1,40}\.gif$", fname):
                return self._send(400, {"error": "非法文件名"})
            f = OUT / fname
            if not f.exists():
                return self._send(404, {"error": "无此 GIF"})
            return self._send(200, f.read_bytes(), "image/gif")
        return self._send(404, {"error": "no route"})

    def do_POST(self):
        u = urlparse(self.path)
        if u.path != "/save":
            return self._send(404, {"error": "no route"})
        try:
            n = int(self.headers.get("Content-Length", "0"))
            req = json.loads(self.rfile.read(n).decode("utf-8"))
            name = req["name"]
            ron = req["ron"]
            render = req.get("render", {})
        except (ValueError, KeyError) as e:
            return self._send(400, {"error": f"请求体无效：{e}"})
        if not NAME_RE.match(name):
            return self._send(400, {"error": f"场景名只许 [a-z0-9_]{{1,40}}：{name!r}"})
        if not ron.endswith("\n"):
            ron += "\n"
        path = SCENARIOS / f"{name}.ron"
        path.write_text(ron, encoding="utf-8", newline="\n")
        OUT.mkdir(exist_ok=True)
        gif = OUT / f"{name}.gif"
        args = ["render", str(path), "-o", str(gif)]
        for key, flag in (("ticks", "--ticks"), ("every", "--every"), ("scale", "--scale"), ("from", "--from")):
            v = render.get(key)
            if v not in (None, ""):
                args += [flag, str(int(v))]
        try:
            rc, out, err = harness(*args, timeout=RENDER_TIMEOUT_S)
        except subprocess.TimeoutExpired:
            return self._send(504, {"error": f"渲染超过 {RENDER_TIMEOUT_S}s 被中止", "log": ""})
        log = (out + err).strip()
        if rc != 0:
            return self._send(500, {"error": "sand-harness render 失败", "log": log})
        return self._send(200, {"gif": f"/out/{gif.name}", "log": log, "saved": str(path.relative_to(ROOT))})

    def log_message(self, fmt, *args):  # 安静一点
        sys.stderr.write("%s %s\n" % (self.address_string(), fmt % args))


def main():
    if not BIN.exists():
        sys.exit(f"找不到 {BIN}\n先跑：cargo build --release -p sand-harness")
    host, port = "127.0.0.1", int(sys.argv[1]) if len(sys.argv) > 1 else 8765
    print(f"map-editor: http://{host}:{port}/  （仓库根 {ROOT}）")
    ThreadingHTTPServer((host, port), Handler).serve_forever()


if __name__ == "__main__":
    main()
