#!/usr/bin/env python3
"""Check refinement regressions through real input in a disposable Studio.

Run after the matrix, before functional_journeys.py changes the fixture.
The app's opt-in report is read-only; screenshots retain original pixels.
"""
import argparse
import json
from pathlib import Path
import subprocess
import time

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--pid', required=True)
p.add_argument('--remote', required=True)
p.add_argument('--report', type=Path, required=True)
p.add_argument('--out', type=Path, required=True)
p.add_argument('--demo', action='store_true')
a = p.parse_args()
a.out.mkdir(parents=True, exist_ok=True)
checks = []


def state():
    return json.loads(a.report.read_text())


def command(*words):
    result = subprocess.run([a.remote, 'app', a.pid, *words],
                            capture_output=True, text=True, check=True)
    for line in result.stdout.splitlines():
        receipt = json.loads(line)
        if 'err' in receipt or receipt.get('verified') is False:
            raise RuntimeError(receipt)
    time.sleep(.3)


def click(name):
    item = state()['controls'][name]
    x, y, w, h = item['rect']
    assert item['visible'] and w > 0 and h > 0, name
    command('click', f'x={x+w/2}', f'y={y+h/2}')


def contained(child, parent):
    controls = state()['controls']
    x, y, w, h = controls[child]['rect']
    u, v, b, c = controls[parent]['rect']
    return (controls[child]['visible'] and w > 0 and h > 0 and
            x >= u-.5 and y >= v-.5 and x+w <= u+b+.5 and y+h <= v+c+.5)


def check(name, result):
    assert result, name
    checks.append({'name': name, 'pass': True})
    (a.out/'checks.json').write_text(json.dumps(checks, indent=2)+'\n')
    print(name, flush=True)


def shot(name):
    command('grab', f'out={(a.out/(name+".png")).resolve()}')


for width, height in [(1440, 900), (1024, 768)]:
    command('window', 'k=resize', f'width={width}', f'height={height}')
    for dark in [False, True]:
        if state()['state']['dark'] != dark:
            click('appearance')
            time.sleep(.7)
        click('nav_record')
        prefix = f'{width}×{height} {"dark" if dark else "light"}'
        base = 'page_record.body.'
        check(prefix+' recording session cues are fully contained',
              contained(base+'right.session.ss_body.cues', base+'right.session'))
        if a.demo:
            check(prefix+' camera waiting text is fully contained',
                  contained(base+'left.cams.cam_0.stage.waiting', base+'left.cams.cam_0.stage'))
            check(prefix+' absent take thumbnails have a visible explanation',
                  contained('page_record.review.rv_body.no_thumbnails', 'page_record.review'))
        else:
            check(prefix+' camera setup guidance is fully contained',
                  contained(base+'left.cams.camera_empty.detail', base+'left.cams.camera_empty'))

if not a.demo:
    command('window', 'k=resize', 'width=1440', 'height=900')
    click('nav_play')
    if not state()['controls']['panel_tools']['visible']:
        click('panels_btn')
    click('layout_reset')
    for columns, label in [(1, 'Column'), (4, 'Row'), (2, 'Grid')]:
        click('layout_cycle')
        check('arrangement button describes '+label,
              state()['state']['panel_layout']['columns'] == columns and
              state()['controls']['layout_cycle']['text'] == 'Layout: '+label)
    shot('selected-panel-controls')
    click('panels_btn')
    for dark in [False, True]:
        if state()['state']['dark'] != dark:
            click('appearance')
            time.sleep(.7)
        x, y, w, h = state()['controls']['speed_btn']['rect']
        command('mouse', 'k=move', f'x={x+w/2}', f'y={y+h/2}')
        shot('toolbar-hover-'+('dark' if dark else 'light'))
        command('mouse', 'k=down', f'x={x+w/2}', f'y={y+h/2}')
        shot('toolbar-pressed-'+('dark' if dark else 'light'))
        command('mouse', 'k=up', f'x={x+w/2}', f'y={y+h/2}')
        command('mouse', 'k=move', 'x=400', 'y=110')
        shot('toolbar-focus-'+('dark' if dark else 'light'))
    # Preserve the functional driver's expected starting transport state.
    for _ in range(4):
        if state()['state']['speed'] == 1:
            break
        click('speed_btn')
    for width, height in [(1440, 900), (1024, 768)]:
        command('window', 'k=resize', f'width={width}', f'height={height}')
        track = state()['controls']['page_play.timeline.tl_body.tl.track_area']
        check(f'{width}×{height} scrub track provides a 44px pointer target',
              track['visible'] and track['rect'][3] >= 44)
else:
    click('nav_hardware')
    command('mouse', 'k=scroll', 'x=900', 'y=620', 'dx=0', 'dy=900')
    actions = 'page_hardware.content.camera_a_panel.content.calib.actions.'
    buttons = [state()['controls'][actions+name] for name in ['btn_restart', 'btn_continue']]
    check('compact calibration scroll reveals both complete action buttons',
          all(b['visible'] and b['rect'][2] > 0 and b['rect'][3] >= 40
              and b['rect'][1] >= 0 and b['rect'][1]+b['rect'][3] <= 756 for b in buttons))
    shot('compact-calibration-actions')
    click('nav_eval')
    command('mouse', 'k=scroll', 'x=450', 'y=650', 'dx=0', 'dy=900')
    shot('compact-evaluation-last-trace')
    click('nav_library')
    command('mouse', 'k=scroll', 'x=600', 'y=600', 'dx=0', 'dy=1600')
    shot('compact-library-bottom')
print(f'{len(checks)} checks passed', flush=True)
