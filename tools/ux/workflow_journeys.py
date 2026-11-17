#!/usr/bin/env python3
"""User-flow acceptance using real Makepad input and read-only telemetry.

Use an isolated Studio instance with DOROBOT_STUDIO_STATE_DIR and disposable
output. The real-errors journey uses missing device paths and a mock serial bus;
it never connects a physical robot. Screenshots and receipts remain local.
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
p.add_argument("--out", type=Path, required=True)
p.add_argument("--mode", choices=["simulation", "real-errors"], default="simulation")
p.add_argument("--keep-running", action="store_true")
a = p.parse_args()
a.out.mkdir(parents=True, exist_ok=True)
receipts = []


def read():
    return json.loads(a.report.read_text())


def command(*args):
    result = subprocess.run(
        [a.remote, "app", a.pid, *args],
        text=True,
        capture_output=True,
        check=True,
        timeout=40,
    )
    values = [json.loads(line) for line in result.stdout.splitlines()]
    for v in values:
        if "err" in v or v.get("verified") is False:
            raise RuntimeError(v)
    time.sleep(0.2)
    return values


def wait(predicate, timeout=20):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = read()
        if predicate(value):
            return value
        time.sleep(0.15)
    raise AssertionError(
        {"message": "UI did not reach expected state", "state": read()["state"]}
    )


def check(name, condition):
    if not condition:
        raise AssertionError(name)
    receipts.append({"check": name, "passed": True})
    print(name, flush=True)


def click(name):
    # 旧版页面意图通过快捷键跳转，避免依赖侧栏分组层级。
    nav = {"nav_hardware": "Key1", "nav_live": "Key2", "nav_record": "Key3",
           "nav_play": "Key4", "nav_library": "Key5", "nav_eval": "Key6"}
    if name in nav:
        command("key", "w=0", "c=" + nav[name], "logo=1", "wait=1")
        return
    wait(
        lambda r: (
            r["controls"][name]["visible"]
            and r["controls"][name]["rect"][2] > 0
            and r["controls"][name]["rect"][3] > 0
        ),
        10,
    )
    c = read()["controls"][name]
    x, y, w, h = c["rect"]
    assert c["visible"] and w > 0 and h > 0 and c["enabled"] is not False, (name, c)
    command("click", "w=0", f"x={x + w / 2}", f"y={y + h / 2}", "wait=1")


def text(name, value):
    click(name)
    command("key", "w=0", "c=KeyA", "logo=1", "wait=1")
    command("text", "w=0", "t=" + value, "wait=1")


def capture(name):
    path = (a.out / (name + ".png")).resolve()
    result = command("grab", "w=0", "out=" + str(path))
    (a.out / (name + ".json")).write_text(json.dumps(read(), indent=2))
    receipts.append(
        {
            "screenshot": path.name,
            "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
            "capture": result,
        }
    )


def phase(name, timeout=30):
    return wait(lambda r: r["state"]["workflow"]["phase"] == name, timeout)


try:
    click("nav_hardware")
    if a.mode == "simulation":
        if not read()["state"]["workflow"]["simulation"]:
            click("wf_sim")
        if not read()["state"]["workflow"]["active"]:
            click("wf_start")
        print("Waiting for physics and cameras…", flush=True)
        wait(lambda r: r["state"]["workflow"]["ready"], 300)
        if read()["state"]["screen"] != "Operate":
            click("wf_operate")
        wait(
            lambda r: (
                r["state"].get("live") and len(r["state"]["live"]["cameras"]) == 3
            ),
            30,
        )
        check(
            "Start simulation reaches Operate with 3 live cameras",
            read()["state"]["screen"] == "Operate",
        )
        capture("operate")
        click("gamepad_setup")
        text("wf_task", "UX validation: capture Nano and all three cameras")
        text("wf_destination", str((a.out / "datasets").resolve()))
        saved_before = read()["state"]["workflow"]["saved"]
        click("wf_record")
        phase("recording")
        wait(lambda r: r["state"]["workflow"]["frames"] >= 8, 60)
        capture("recording")
        click("nav_hardware")
        check(
            "Setup preserves recording and blocks simulation shutdown",
            read()["state"]["workflow"]["phase"] == "recording"
            and read()["controls"]["wf_stop"]["enabled"] is False,
        )
        click("nav_library")
        check(
            "Library navigation preserves recording",
            read()["state"]["workflow"]["phase"] == "recording",
        )
        click("nav_record")
        click("wf_finish")
        phase("review", 60)
        check(
            "Stop waits for every sensor frame",
            "Episode ready" in read()["state"]["workflow"]["detail"],
        )
        count = read()["state"]["workflow"]["frames"]
        capture("review")
        click("nav_library")
        click("nav_record")
        check(
            "Unsaved take survives navigation",
            read()["state"]["workflow"]["phase"] == "review"
            and read()["state"]["workflow"]["frames"] == count,
        )
        click("wf_save")
        phase("saved", 60)
        dataset = Path(read()["state"]["workflow"]["dataset"])
        check(
            "Save commits a discoverable dataset",
            dataset.joinpath("meta/info.json").is_file(),
        )
        (a.out / "dataset-path.txt").write_text(str(dataset))
        click("wf_replay")
        wait(
            lambda r: r["state"]["screen"] == "Replay" and r["state"]["frames"] == count
        )
        check(
            "Replay loads all 17 measured joints",
            len(read()["state"]["recorded_joints"]) == 17,
        )
        click("play_btn")
        wait(lambda r: r["state"]["time"] > 0.3)
        click("play_btn")
        capture("saved-replay")
        check(
            "Saved replay advances independently of the live source",
            read()["state"]["time"] > 0.3 and read()["state"]["workflow"]["active"],
        )
        click("nav_record")
        click("wf_record")
        phase("recording")
        wait(lambda r: r["state"]["workflow"]["frames"] >= 3, 30)
        click("wf_finish")
        phase("review", 30)
        click("wf_discard")
        phase("idle")
        check(
            "Discard removes only the new take and preserves saved data",
            dataset.joinpath("meta/info.json").is_file()
            and read()["state"]["workflow"]["saved"] == saved_before + 1,
        )
        if not a.keep_running:
            click("nav_hardware")
            click("wf_stop")
            wait(lambda r: not r["state"]["workflow"]["active"], 25)
            check(
                "Explicit simulation stop is reported as normal completion",
                "Simulation stopped." in read()["state"]["workflow"]["detail"],
            )
    else:
        click("wf_real")
        click("wf_arm")
        c = read()["controls"]["wf_edit"]
        if c["visible"] and c["rect"][2] > 0:
            click("wf_edit")
        text("wf_type", "wrong")
        click("wf_create")
        check(
            "Invalid profile gives an actionable error",
            "so100 or so101" in read()["state"]["status"],
        )
        text("wf_type", "so101")
        text("wf_follower", "/dev/dorobot-review-missing-follower")
        text("wf_leader", "/dev/dorobot-review-missing-leader")
        text("wf_cameras", "0,1")
        text("wf_destination", str((a.out / "real-dataset").resolve()))
        capture("real-profile-form")
        click("wf_create")
        click("page_hardware.content.camera_a_panel.content.calib.actions.btn_continue")
        wait(lambda r: "device not found" in r["state"]["status"])
        check(
            "Missing hardware is blocked before calibration",
            not read()["state"]["workflow"]["calibrated"],
        )
        capture("missing-device")
        click("nav_record")
        check(
            "Physical capture is gated by calibration",
            read()["controls"]["page_record.transport.btn_record"]["enabled"] is False,
        )
        click("nav_hardware")
        click("wf_nano")
        check(
            "Nano capabilities explicitly exclude arm calibration and cameras",
            read()["controls"]["wf_detail"]["visible"]
            and "arms, head and cameras unavailable" in read()["controls"]["wf_detail"]["text"],
        )
        capture("nano-base-setup")
        click(
            "page_hardware.content.camera_a_panel.content.calib.nano_controls.connection_row.test"
        )
        wait(lambda r: r["state"]["workflow"].get("nano", {}).get("acks", 0) > 0, 40)
        click("wf_operate")
        check(
            "Nano mock base remains connected in Operate",
            read()["state"]["screen"] == "Operate"
            and read()["state"]["workflow"]["nano"]["active"],
        )
        capture("nano-bus-controls")
        click("nav_library")
        wait(lambda r: not r["state"]["workflow"]["nano"]["active"], 15)
        check(
            "Leaving physical controls stops the base driver",
            not read()["state"]["workflow"]["nano"]["active"],
        )
    (a.out / (a.mode + "-receipts.json")).write_text(
        json.dumps({"passed": True, "checks": receipts}, indent=2)
    )
except Exception as error:
    (a.out / (a.mode + "-receipts.json")).write_text(
        json.dumps({"passed": False, "error": str(error), "checks": receipts}, indent=2)
    )
    raise
