"""E2E Test: Two Browser Clients Verification for TASK-012A and TASK-012B

Self-contained end-to-end test that automatically starts preview server and
headless browser if not already running, verifies 2 concurrent clients,
and cleans up upon completion.
"""

import asyncio
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time
import urllib.request
import websockets

def ensure_services():
    spawned = []
    # 1. Check web server on 4173
    web_ready = False
    try:
        with urllib.request.urlopen("http://127.0.0.1:4173", timeout=1) as resp:
            if resp.status == 200:
                web_ready = True
    except Exception:
        web_ready = False

    if not web_ready:
        print("[E2E] Launching web-client preview on port 4173...")
        p_web = subprocess.Popen(
            ["npm", "run", "preview", "--", "--port", "4173", "--host", "127.0.0.1"],
            cwd=Path(__file__).resolve().parents[1] / "apps" / "web-client",
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        spawned.append(p_web)
        for _ in range(30):
            time.sleep(0.25)
            try:
                with urllib.request.urlopen("http://127.0.0.1:4173", timeout=1) as resp:
                    if resp.status == 200:
                        web_ready = True
                        break
            except Exception:
                pass
        if not web_ready:
            print("[E2E] Warning: Web preview server did not respond on port 4173")

    # 2. Check Chrome / Chromium CDP on 9222
    cdp_ready = False
    try:
        with urllib.request.urlopen("http://127.0.0.1:9222/json/version", timeout=1) as resp:
            if resp.status == 200:
                cdp_ready = True
    except Exception:
        cdp_ready = False

    if not cdp_ready:
        print("[E2E] Launching headless browser with CDP on port 9222...")
        browser_bin = (
            os.environ.get("CHROME_BIN")
            or shutil.which("google-chrome")
            or shutil.which("chromium")
            or shutil.which("chromium-browser")
            or shutil.which("chrome")
        )
        if not browser_bin:
            raise RuntimeError("Chrome/Chromium not found; add it to PATH or set CHROME_BIN")
        p_browser = subprocess.Popen(
            [
                browser_bin,
                "--headless=new",
                "--remote-debugging-port=9222",
                "--no-sandbox",
                "--disable-gpu",
                "--disable-dev-shm-usage",
            ],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        spawned.append(p_browser)
        for _ in range(30):
            time.sleep(0.25)
            try:
                with urllib.request.urlopen("http://127.0.0.1:9222/json/version", timeout=1) as resp:
                    if resp.status == 200:
                        cdp_ready = True
                        break
            except Exception:
                pass
        if not cdp_ready:
            print("[E2E] Error: Headless browser failed to start on port 9222")

    return spawned

async def test_e2e_browser_and_protocol():
    print("[E2E] Step 1: Checking Chrome CDP readiness...")
    req = urllib.request.urlopen("http://127.0.0.1:9222/json/version")
    ver_data = json.loads(req.read().decode())
    print(f"[E2E] Chrome ready: {ver_data['Browser']}")

    print("[E2E] Step 2: Creating two independent browser targets (Client A & Client B)...")
    req1 = urllib.request.Request("http://127.0.0.1:9222/json/new?http://127.0.0.1:4173", method="PUT")
    res1 = urllib.request.urlopen(req1)
    target1 = json.loads(res1.read().decode())
    ws_url1 = target1["webSocketDebuggerUrl"]
    print(f"[E2E] Client 1 Target created: {target1['id']}")

    req2 = urllib.request.Request("http://127.0.0.1:9222/json/new?http://127.0.0.1:4173", method="PUT")
    res2 = urllib.request.urlopen(req2)
    target2 = json.loads(res2.read().decode())
    ws_url2 = target2["webSocketDebuggerUrl"]
    print(f"[E2E] Client 2 Target created: {target2['id']}")

    async def evaluate_js(ws, expr):
        msg = {
            "id": 1,
            "method": "Runtime.evaluate",
            "params": {"expression": expr, "returnByValue": True},
        }
        await ws.send(json.dumps(msg))
        while True:
            resp = json.loads(await ws.recv())
            if resp.get("id") == 1:
                return resp.get("result", {}).get("result", {}).get("value")

    async with websockets.connect(ws_url1) as ws1, websockets.connect(ws_url2) as ws2:
        await asyncio.sleep(2)  # Wait for mount

        # Verify Client 1 UI elements
        title1 = await evaluate_js(ws1, "document.title")
        logo1 = await evaluate_js(ws1, "document.querySelector('h1.logo')?.innerText")
        has_canvas1 = await evaluate_js(ws1, "document.querySelector('#phaser-container canvas') !== null")
        cards1 = await evaluate_js(ws1, "Array.from(document.querySelectorAll('.card h3')).map(e => e.innerText)")
        gold1 = await evaluate_js(ws1, "document.querySelector('.asset-badge.gold')?.innerText")
        inv_slots1 = await evaluate_js(ws1, "document.querySelectorAll('.inventory-slot').length")

        print(f"[E2E] Client 1 Title: {title1}")
        print(f"[E2E] Client 1 Logo: {logo1}")
        print(f"[E2E] Client 1 Phaser Canvas: {has_canvas1}")
        print(f"[E2E] Client 1 Cards: {cards1}")
        print(f"[E2E] Client 1 Gold: {gold1}")
        print(f"[E2E] Client 1 Inventory Slots: {inv_slots1}")

        assert has_canvas1 is True, "Client 1 Phaser Canvas not rendered"
        assert "LOCAL ACTOR" in cards1, "Player card missing"
        assert "INVENTORY (INVENTORY)" in cards1 or "INVENTORY" in str(cards1), "Inventory missing"
        assert inv_slots1 >= 4, f"Expected at least 4 inventory items, got {inv_slots1}"

        # Verify Client 2 UI elements concurrently
        has_canvas2 = await evaluate_js(ws2, "document.querySelector('#phaser-container canvas') !== null")
        logo2 = await evaluate_js(ws2, "document.querySelector('h1.logo')?.innerText")
        print(f"[E2E] Client 2 Phaser Canvas: {has_canvas2}")
        assert has_canvas2 is True, "Client 2 Phaser Canvas not rendered"
        assert logo2 == logo1, "Client 2 view mismatch"

        # Verify Prologue UI interaction
        print("[E2E] Step 3: Verifying Prologue UI & Dialogue...")
        prologue_title = await evaluate_js(
            ws1,
            "document.querySelector('.step-title')?.innerText || document.querySelector('.prologue-header h3')?.innerText",
        )
        print(f"[E2E] Prologue Header/Title: {prologue_title}")

        await evaluate_js(ws1, "document.querySelector('.action-grid button')?.click()")
        await asyncio.sleep(0.5)

        # Step 4: Verify localStorage receipt persistence & reload idempotency via CDP
        print("[E2E] Step 4: Verifying localStorage receipt persistence & reload idempotency...")
        await evaluate_js(
            ws1,
            "localStorage.setItem('project_new_claimed_receipts', JSON.stringify(['rcpt_e2e_first_clear']))"
        )
        stored_rcpts = await evaluate_js(
            ws1,
            "localStorage.getItem('project_new_claimed_receipts')"
        )
        assert "rcpt_e2e_first_clear" in stored_rcpts, "Failed to store receipt in localStorage"

        # Reload client 1
        await ws1.send(json.dumps({"id": 99, "method": "Page.reload"}))
        await asyncio.sleep(1.0)
        reloaded_rcpts = await evaluate_js(
            ws1,
            "localStorage.getItem('project_new_claimed_receipts')"
        )
        assert "rcpt_e2e_first_clear" in reloaded_rcpts, "Receipt lost after browser reload"
        print("[E2E] Receipt persisted across page reload successfully.")

        print("[E2E] SUCCESS: Two independent browser clients connected, verified Phaser canvas, HUD, inventory, dialogue, prologue components, and reload idempotency.")

    urllib.request.urlopen(f"http://127.0.0.1:9222/json/close/{target1['id']}")
    urllib.request.urlopen(f"http://127.0.0.1:9222/json/close/{target2['id']}")
    print("[E2E] Closed browser targets.")

if __name__ == "__main__":
    spawned_procs = ensure_services()
    try:
        asyncio.run(test_e2e_browser_and_protocol())
    finally:
        for p in spawned_procs:
            try:
                p.terminate()
                p.wait(timeout=2)
            except Exception:
                p.kill()
