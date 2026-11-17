#!/usr/bin/env python3
"""Exercise language switching through actual Makepad input in an isolated app.

Launch with a disposable DOROBOT_STUDIO_STATE_DIR, DOROBOT_UX_REPORT and a
copied Nano dataset named Record whose task is 'Record / 中文任务 {0}'. Never
point this acceptance run at an active user session. No real hardware is opened.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import time

p = argparse.ArgumentParser(description=__doc__)
p.add_argument("--pid", required=True)
p.add_argument("--remote", required=True)
p.add_argument("--report", type=Path, required=True)
p.add_argument("--state-dir", type=Path, required=True)
p.add_argument("--out", type=Path, required=True)
p.add_argument("--restore-only", action="store_true")
a = p.parse_args()
a.out.mkdir(parents=True, exist_ok=True)
checks = []


def read():
    return json.loads(a.report.read_text())


def wait(predicate, timeout=20):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        try:
            r = read()
            if predicate(r):
                return r
        except (FileNotFoundError, json.JSONDecodeError):
            pass
        time.sleep(.15)
    raise AssertionError("UI state did not arrive: " + str(read()["state"]))


def command(*args):
    result = subprocess.run([a.remote, "app", a.pid, *args], check=True,
                            capture_output=True, text=True, timeout=40)
    values = [json.loads(line) for line in result.stdout.splitlines()]
    assert all("err" not in v and v.get("verified") is not False for v in values), values
    time.sleep(.2)
    return values


def click(name, window=0):
    # 旧版页面意图通过快捷键跳转，避免依赖侧栏分组层级。
    nav = {"nav_hardware": "Key1", "nav_live": "Key2", "nav_record": "Key3",
           "nav_play": "Key4", "nav_library": "Key5", "nav_eval": "Key6"}
    if name in nav:
        command("key", f"w={window}", "c=" + nav[name], "logo=1", "wait=1")
        return
    r = wait(lambda r: r["controls"][name]["visible"] and r["controls"][name]["rect"][2] > 0)
    c = r["controls"][name]
    assert c["enabled"] is not False, name
    x, y, w, h = c["rect"]
    command("click", f"w={window}", f"x={x+w/2}", f"y={y+h/2}", "wait=1")


def enter(name, value):
    click(name)
    command("key", "w=0", "c=KeyA", "logo=1", "wait=1")
    command("text", "w=0", "t=" + value, "wait=1")


def check(name, ok):
    assert ok, name
    checks.append({"check": name, "passed": True})
    print(name, flush=True)


def language(value):
    if wait(lambda r: "language" in r["state"])["state"]["language"] != value:
        click("language_toggle")
    wait(lambda r: r["state"]["language"] == value)


def capture(name, window=0):
    path = a.out / (name + ".png")
    command("grab", f"w={window}", "out=" + str(path.resolve()))
    (a.out / (name + ".json")).write_text(json.dumps(read(), ensure_ascii=False, indent=2))
    return hashlib.sha256(path.read_bytes()).hexdigest()


try:
    if a.restore_only:
        r = wait(lambda r: r["state"]["language"] == "zh-CN")
        check("Chinese preference survives a fresh app process", r["controls"]["data_workspace"]["text"] == "数据工作区")
        check("Restoring language does not start hardware or simulation", not r["state"]["workflow"]["active"] and not r["state"]["recording"])
        capture("restored-zh")
    else:
        language("en")
        check("English labels render", read()["controls"]["data_workspace"]["text"] == "Data workspace")
        language("zh-CN")
        for nav, screen, title, tab, caption in [
            ("nav_hardware", "Robot setup", "机器人控制", "workspace_tabs.plugin_tabs.plugin_tab_page_hardware", "机器人设置"),
            ("nav_live", "Operate", "机器人控制", "workspace_tabs.plugin_tabs.plugin_tab_hardware_teleop", "操作"),
            ("nav_record", "Record", "数据采集", "workspace_tabs.plugin_tabs.plugin_tab_page_record", "录制"),
            ("nav_play", "Replay", "数据工作区", "workspace_tabs.data_tabs.tab_play", "回放"),
            ("nav_library", "Library", "数据工作区", "workspace_tabs.data_tabs.tab_library", "数据集"),
            ("nav_eval", "Eval", "数据工作区", "workspace_tabs.data_tabs.tab_eval", "评估"),
        ]:
            click(nav)
            r = wait(lambda r: r["state"]["screen"] == screen
                     and r["controls"]["main_window.body.workspace.page_header.page_title"]["text"] == title
                     and r["controls"][tab]["visible"] and r["controls"][tab]["text"] == caption)
            check("Chinese navigation: " + nav, True)
        click("nav_library")
        check("A dataset named Record remains user text", read()["controls"]["page_library.left_col.grid.row_a.card_0.body.name"]["text"] == "Record")
        capture("library-zh")
        click("nav_play")
        click("details_btn")
        check("Recorded task and braces are preserved", read()["controls"]["page_play.upper.side.side_body.task_t"]["text"] == "Record / 中文任务 {0}")
        click("robot_panel.head.maximize")
        before = read()["state"]["panel_layout"]
        click("speed_btn")  # 2x
        click("speed_btn")  # 0.25x
        command("key", "w=0", "c=Home", "wait=1")
        click("play_btn")
        wait(lambda r: r["state"]["playing"] and r["state"]["time"] > .1)
        old = read()["state"]
        language("en")
        r = wait(lambda r: r["state"]["time"] > old["time"])
        check("Switching language preserves playback and the episode", r["state"]["playing"] and r["state"]["episode"] == old["episode"])
        check("Switching language preserves a maximized panel", r["state"]["panel_layout"] == before)
        language("zh-CN")
        click("play_btn")
        command("key", "w=0", "c=Escape", "wait=1")
        click("panels_btn")
        click("new_window")
        wait(lambda r: r["state"]["monitor_open"])
        monitor = read()["state"]["monitor_layout"]
        language("en")
        language("zh-CN")
        check("The second replay window is translated without losing its layout",
              read()["controls"]["monitor_play.toolbar.panels_btn"]["text"] == "面板"
              and read()["state"]["monitor_layout"] == monitor)
        capture("replay-zh")
        capture("monitor-zh", read()["state"]["monitor_window"])
        click("close_monitor", read()["state"]["monitor_window"])
        click("appearance")
        capture("replay-zh-dark")
        click("appearance")
        click("nav_hardware")
        if not read()["state"]["workflow"]["simulation"]:
            click("wf_sim")
        click("wf_start")
        wait(lambda r: r["state"]["workflow"]["ready"], 300)
        wait(lambda r: r["state"].get("live") and len(r["state"]["live"]["cameras"]) == 3, 60)
        r = wait(lambda r: "实时 · 仿真" in r["controls"]["main_window.body.workspace.page_header.status"]["text"])
        status = r["controls"]["main_window.body.workspace.page_header.status"]["text"]
        check("Live status components are translated", "实时 · 仿真" in status and "Simulation" not in status)
        capture("operate-zh")
        click("nav_record")
        destination = str((a.out / "datasets").resolve())
        enter("wf_task", "Record")
        enter("wf_destination", destination)
        language("en")
        language("zh-CN")
        check("Language switching preserves editable task and destination",
              read()["controls"]["wf_task"]["text"] == "Record" and read()["controls"]["wf_destination"]["text"] == destination)
        click("wf_record")
        wait(lambda r: r["state"]["workflow"]["phase"] == "recording" and r["state"]["workflow"]["frames"] >= 4, 60)
        frames = read()["state"]["workflow"]["frames"]
        language("en")
        language("zh-CN")
        wait(lambda r: r["state"]["workflow"]["frames"] > frames, 30)
        check("Active recording survives both language switches", read()["state"]["workflow"]["phase"] == "recording")
        capture("recording-zh")
        click("wf_finish")
        wait(lambda r: r["state"]["workflow"]["phase"] == "review", 60)
        frames = read()["state"]["workflow"]["frames"]
        language("en")
        language("zh-CN")
        check("Unsaved take survives language switching", read()["state"]["workflow"]["frames"] == frames and read()["state"]["workflow"]["phase"] == "review")
        click("wf_save")
        r = wait(lambda r: r["state"]["workflow"]["phase"] == "saved", 60)
        dataset = Path(r["state"]["workflow"]["dataset"])
        tasks = [json.loads(s) for s in (dataset / "meta/tasks.jsonl").read_text().splitlines()]
        check("Saved task stays Record, not a translated label", tasks[0]["task"] == "Record")
        click("nav_hardware")
        click("wf_stop")
        wait(lambda r: not r["state"]["workflow"]["active"], 30)
        stored = json.loads((a.state_dir / "session.json").read_text())
        check("Language preference is saved", stored["language"] == "zh-CN")
    (a.out / ("restore-receipts.json" if a.restore_only else "i18n-receipts.json")).write_text(
        json.dumps({"passed": True, "checks": checks}, indent=2))
except Exception as error:
    (a.out / "failure.json").write_text(json.dumps({"passed": False, "error": str(error), "checks": checks}, indent=2))
    raise
