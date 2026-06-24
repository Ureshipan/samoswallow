#!/usr/bin/env python3
"""
samoswallow demo app — a tiny zero-dependency web service that lets you exercise
every feature of the platform from the browser:

  /            dashboard: instance id, env var, uptime, request count
  /health      health check endpoint (used by swallow.yaml healthcheck)
  /load?s=3    burn CPU for N seconds  -> watch CPU% climb in the UI
  /mem?mb=50   hold N MB of memory     -> watch RAM climb in the UI
  /crash       exit the process        -> watch samoswallow auto-restart it
  /env         dump environment as JSON

Reads two env vars (set them in the samoswallow UI → Secrets/Env to see them
change live): GREETING and PORT.
"""
import json
import os
import socket
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlparse, parse_qs

START = time.monotonic()
HOSTNAME = socket.gethostname()  # container id, so you can tell instances apart
GREETING = os.environ.get("GREETING", "Привет из samoswallow!")
PORT = int(os.environ.get("PORT", "8080"))

_requests = 0
_lock = threading.Lock()
_mem_hold = []  # keeps allocated buffers alive for /mem


def uptime():
    return round(time.monotonic() - START, 1)


PAGE = """<!doctype html>
<html lang="ru"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>samoswallow demo</title>
<style>
 body {{ font-family: ui-sans-serif, system-ui, sans-serif; background:#14161a; color:#e6e6e6;
        max-width:640px; margin:40px auto; padding:0 16px; }}
 h1 {{ color:#f5c542; }} a,button {{ color:#6fb3ff; }}
 .card {{ background:#1d2026; border:1px solid #2a2e36; border-radius:10px; padding:16px 18px; margin:14px 0; }}
 .k {{ color:#9aa3af; }} code {{ background:#0f1115; padding:2px 6px; border-radius:6px; }}
 button {{ font:inherit; cursor:pointer; background:#262a32; color:#e6e6e6; border:1px solid #3a3f49;
           border-radius:8px; padding:8px 12px; margin:4px 4px 0 0; }}
 #out {{ white-space:pre-wrap; font-family:ui-monospace,monospace; font-size:13px; color:#6ee787; }}
</style></head><body>
<h1>🚛 {greeting}</h1>
<div class="card">
  <div><span class="k">Инстанс (hostname):</span> <code>{host}</code></div>
  <div><span class="k">Uptime:</span> {uptime} c</div>
  <div><span class="k">Обработано запросов:</span> {reqs}</div>
  <div><span class="k">Порт:</span> {port}</div>
</div>
<div class="card">
  <p>Понагружай инстанс и смотри метрики в самосвале:</p>
  <button onclick="hit('/load?s=3')">CPU нагрузка (3с)</button>
  <button onclick="hit('/mem?mb=50')">Занять +50 МБ</button>
  <button onclick="hit('/health')">Health</button>
  <button onclick="hit('/env')">Env (JSON)</button>
  <button onclick="if(confirm('Уронить процесс? Самосвал должен перезапустить инстанс.')) hit('/crash')">💥 Crash</button>
  <p id="out"></p>
</div>
<script>
async function hit(p) {{
  const o = document.getElementById('out');
  o.textContent = '… ' + p;
  try {{ const r = await fetch(p); o.textContent = p + ' -> ' + r.status + '\\n' + await r.text(); }}
  catch (e) {{ o.textContent = p + ' -> ' + e; }}
}}
</script>
</body></html>"""


class Handler(BaseHTTPRequestHandler):
    def _send(self, code, body, ctype="text/plain; charset=utf-8"):
        data = body.encode() if isinstance(body, str) else body
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        global _requests
        with _lock:
            _requests += 1
        url = urlparse(self.path)
        q = parse_qs(url.query)

        if url.path == "/":
            self._send(200, PAGE.format(greeting=GREETING, host=HOSTNAME,
                                        uptime=uptime(), reqs=_requests, port=PORT),
                       "text/html; charset=utf-8")
        elif url.path == "/health":
            self._send(200, "ok")
        elif url.path == "/load":
            seconds = min(float(q.get("s", ["3"])[0]), 30)
            end = time.monotonic() + seconds
            while time.monotonic() < end:
                _ = sum(i * i for i in range(10000))
            self._send(200, f"burned CPU for {seconds}s")
        elif url.path == "/mem":
            mb = min(int(q.get("mb", ["50"])[0]), 1024)
            _mem_hold.append(bytearray(mb * 1024 * 1024))
            held = sum(len(b) for b in _mem_hold) // (1024 * 1024)
            self._send(200, f"allocated +{mb}MB (holding {held}MB total)")
        elif url.path == "/env":
            self._send(200, json.dumps(dict(os.environ), indent=2, ensure_ascii=False),
                       "application/json; charset=utf-8")
        elif url.path == "/crash":
            self._send(200, "crashing now — samoswallow should restart this instance")
            os._exit(1)
        else:
            self._send(404, "not found")

    def log_message(self, fmt, *args):
        # Log to stdout so the log view in samoswallow shows traffic.
        print(f"{self.address_string()} {fmt % args}", flush=True)


if __name__ == "__main__":
    print(f"demo listening on :{PORT}  (instance {HOSTNAME})", flush=True)
    ThreadingHTTPServer(("0.0.0.0", PORT), Handler).serve_forever()
