#!/usr/bin/env python3
"""macOS native picker cancellation on a visible isolated test PID (requires existing AX access)."""
import argparse, json, subprocess, time
from pathlib import Path
p=argparse.ArgumentParser(description=__doc__)
p.add_argument('--pid',type=int,required=True);p.add_argument('--remote',required=True)
p.add_argument('--report',type=Path,required=True);p.add_argument('--out',type=Path,required=True)
a=p.parse_args();checks=[]
def read():return json.loads(a.report.read_text())
def cmd(*args):
 r=subprocess.run([a.remote,'app',str(a.pid),*args,'w=0'],capture_output=True,text=True,check=True)
 for line in r.stdout.splitlines():
  value=json.loads(line)
  if 'err' in value or value.get('verified') is False:raise RuntimeError(value)
 time.sleep(.3)
def click(name):
 x,y,w,h=read()['controls'][name]['rect'];assert w>0 and h>0
 cmd('click',f'x={x+w/2}',f'y={y+h/2}')
def apple(code):return subprocess.check_output(['osascript','-e',code],text=True,timeout=15).strip()
def wait(test):
 end=time.monotonic()+10
 while time.monotonic()<end:
  if test():return
  time.sleep(.1)
 raise AssertionError('native dialog callback did not return')
def task():
 r=read()['state'];return [r['episode'],r['time'],r['panel_layout']]
def window_count():
 return int(apple(f'tell application "System Events" to tell (first application process whose unix id is {a.pid}) to count windows'))
def cancel():
 apple(f'tell application "System Events"\ntell (first application process whose unix id is {a.pid})\nset frontmost to true\nkey code 53\nend tell\nend tell')
previous=apple('tell application "System Events" to get unix id of first application process whose frontmost is true')
try:
 click('nav_library');before=task();count=window_count();click('page_library.left_col.header.btn_open')
 wait(lambda:read()['state']['folder_pending']);wait(lambda:window_count()>count)
 cancel();time.sleep(2.2);wait(lambda:not read()['state']['folder_pending'])
 assert task()==before
 checks.append({'name':'Cancel the native folder picker without changing episode, playhead or layout','pass':True})
 click('nav_record');before=task();count=window_count();click('setup_rig')
 wait(lambda:window_count()>count);cancel();time.sleep(2.2);wait(lambda:window_count()<=count)
 assert task()==before and not read()['state']['recording']
 checks.append({'name':'Cancel the native rig-profile picker without configuring or starting hardware','pass':True})
 click('nav_play')
finally:
 if previous.isdigit():
  apple(f'tell application "System Events" to set frontmost of (first application process whose unix id is {int(previous)}) to true')
a.out.parent.mkdir(parents=True,exist_ok=True);a.out.write_text(json.dumps(checks,indent=2)+'\n')
print(f'{len(checks)} native dialog checks passed')
