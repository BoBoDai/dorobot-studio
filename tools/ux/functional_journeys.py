#!/usr/bin/env python3
"""Functional parity gate; real remote input, read-only app observations.
Use only an isolated app and disposable dataset. Results stay outside Git.
"""
import argparse, hashlib, json, subprocess, time
from pathlib import Path
p=argparse.ArgumentParser(description=__doc__)
p.add_argument('--pid',required=True);p.add_argument('--remote',required=True)
p.add_argument('--report',type=Path,required=True);p.add_argument('--out',type=Path,required=True)
p.add_argument('--mode',choices=['replay','demo','restart','layout','speed'],default='replay')
a=p.parse_args();a.out.mkdir(parents=True,exist_ok=True);checks=[]
def cmd(*args,w=0):
 r=subprocess.run([a.remote,'app',a.pid,*args,f'w={w}'],capture_output=True,text=True,check=True)
 for line in r.stdout.splitlines():
  value=json.loads(line)
  if 'err' in value or value.get('verified') is False:raise RuntimeError(value)
 time.sleep(.18)
 return r.stdout

def read():return json.loads(a.report.read_text())
def wait(test):
 end=time.monotonic()+15
 while time.monotonic()<end:
  r=read()
  if test(r):return r
  time.sleep(.1)
 raise AssertionError('State did not settle')
def rect(n):return read()['controls'][n]['rect']
def click(n,w=0):
 c=read()['controls'][n];x,y,b,h=c['rect'];assert c['visible'] and b>0 and h>0,n
 cmd('click',f'x={x+b/2}',f'y={y+h/2}',w=w)
def drag(x,y,u,v,w=0):
 cmd('mouse','k=down',f'x={x}',f'y={y}',w=w)
 for i in range(1,6):cmd('mouse','k=move',f'x={x+(u-x)*i/5}',f'y={y+(v-y)*i/5}',w=w)
 cmd('mouse','k=up',f'x={u}',f'y={v}',w=w)
def key(k,**mods):cmd('key',f'c={k}',*[f'{k}={v}' for k,v in mods.items()])
def check(name,condition):
 assert condition,name
 checks.append({'name':name,'pass':True});print(name,flush=True)
 (a.out/'checks.json').write_text(json.dumps(checks,indent=2)+'\n')
def shot(name,w=0):
 path=(a.out/(name+'.png')).resolve();cmd('grab',f'out={path}',w=w)
 (a.out/(name+'.sha256')).write_text(hashlib.sha256(path.read_bytes()).hexdigest()+'\n')
def nav(n):click('nav_'+n);time.sleep(.25)
def layout():return read()['state']['panel_layout']
def contained(child,parent):
 x,y,w,h=rect(child);a,b,c,d=rect(parent)
 return w>0 and h>0 and x>=a-.5 and y>=b-.5 and x+w<=a+c+.5 and y+h<=b+d+.5

if a.mode=='speed':
 nav('play')
 if not read()['controls']['panel_tools']['visible']:click('panels_btn')
 click('new_window');wait(lambda r:r['state']['monitor_open'])
 for _ in range(4):
  if read()['state']['speed']==1:break
  click('speed_btn')
 for speed in [2,.25,.5,1]:
  click('speed_btn');wait(lambda r:r['state']['speed']==speed)
  check(f'{speed}x playback speed is shared by both windows',read()['controls']['speed_btn']['text']==read()['controls']['monitor_play.speed_btn']['text'])
 click('speed_btn');key('Home');click('play_btn');wait(lambda r:r['state']['playing'])
 start=read()['state']['time'];at=time.monotonic();time.sleep(1.5)
 rate=(read()['state']['time']-start)/(time.monotonic()-at);click('play_btn');wait(lambda r:not r['state']['playing'])
 check('2x control changes actual playback rate',1.65<rate<2.35)
 click('close_monitor',w=read()['state']['monitor_window']);click('panels_btn')
 if not read()['controls']['page_play.upper.side']['visible']:click('details_btn')
 x,y,w,h=rect('page_play.upper.side');cmd('mouse','k=scroll',f'x={x+w/2}',f'y={y+h/2}','dx=0','dy=-500')
 if read()['state']['tag']=='Some(Good)':click('page_play.upper.side.side_body.btn_good')
 click('page_play.upper.side.side_body.btn_good');wait(lambda r:r['state']['tag']=='Some(Good)')
 nav('library');nav('play');check('good tag survives navigation',read()['state']['tag']=='Some(Good)')
 x,y,w,h=rect('page_play.upper.side');cmd('mouse','k=scroll',f'x={x+w/2}',f'y={y+h/2}','dx=0','dy=500')
 click('page_play.upper.side.side_body.btn_push');wait(lambda r:r['state']['hub_open'])
 click('hub_cancel');wait(lambda r:not r['state']['hub_open'])
 check('cancel upload form retains episode and tag',read()['state']['tag']=='Some(Good)')
 shot('shared-speed-control')
elif a.mode=='layout':
 nav('play')
 if not read()['controls']['panel_tools']['visible']:click('panels_btn')
 click('layout_reset')
 for name in ['robot_toggle','camera_a_toggle','camera_b_toggle','error_toggle']:click(name)
 wait(lambda r:r['state']['panel_layout']['visible']==[False]*4)
 check('all closed panels leave recovery controls available',read()['controls']['layout_reset']['visible'])
 shot('all-panels-closed')
 click('layout_reset');wait(lambda r:all(r['state']['panel_layout']['visible']))
 check('reset restores every view after closing all panels',all(read()['controls'][n]['visible'] for n in ['robot_panel','camera_a_panel','camera_b_panel','error_panel']))
 for columns in [1,4,2]:
  click('layout_cycle');wait(lambda r:r['state']['panel_layout']['columns']==columns)
  check(f'arrangement with {columns} columns retains panel content',contained('stage_3d','robot_panel') and contained('stage_plot','error_panel'))
 click('panels_btn');cmd('window','k=resize','width=1024','height=768');time.sleep(.4)
 if not read()['state']['signals_visible']:click('signals_btn')
 for _ in range(5):click('signal_next')
 check('compact signal paging reaches the last existing lane',read()['controls']['page_play.timeline.tl_body.lanes.content.lane_5']['visible'])
 shot('compact-last-signal')
 click('signals_btn')
 for dark in [False,True]:
  if read()['state']['dark']!=dark:click('appearance')
  click('details_btn') if not read()['controls']['page_play.upper.side']['visible'] else None
  shot('compact-details-'+('dark' if dark else 'light'))
elif a.mode=='restart':
 expected=json.loads((a.out.parent/'expected-restart.json').read_text())
 wait(lambda r:r['state']['screen']=='Replay')
 check('restart restores panel order, sizes and visibility',layout()==expected['layout'])
 check('restart restores episode and paused playhead',read()['state']['episode']==expected['episode'] and abs(read()['state']['time']-expected['time'])<.01 and not read()['state']['playing'])
 if 'speed' in expected:check('restart restores playback speed without starting playback',read()['state']['speed']==expected['speed'] and not read()['state']['playing'])
 check('restart preserves curation tag',read()['state']['tag']==expected['tag'])
 check('restart restores navigation width',abs(rect('sidebar')[2]-expected['sidebar_width'])<1)
 shot('restored-layout')
elif a.mode=='demo':
 nav('record');wait(lambda r:r['controls']['page_record.transport.btn_save']['enabled'] is True)
 saved=read()['state']['saved'];click('page_record.transport.btn_save')
 wait(lambda r:r['state']['saved']==saved+1)
 check('simulated completed take saves and clears review',read()['controls']['page_record.transport.btn_record']['enabled'] and not read()['controls']['page_record.transport.btn_save']['enabled'])
 click('page_record.transport.btn_record');wait(lambda r:r['state']['recording'])
 check('simulated record starts with an available Stop',read()['controls']['page_record.transport.btn_record']['text']=='Stop' and read()['controls']['page_record.transport.btn_record']['enabled'])
 click('page_record.transport.btn_record');wait(lambda r:not r['state']['recording'])
 check('simulated stop presents Save, Discard and Re-record',all(read()['controls']['page_record.transport.'+n]['enabled'] for n in ['btn_save','btn_discard','btn_rerecord']))
 shot('simulated-take-review')
 click('page_record.transport.btn_rerecord');wait(lambda r:r['state']['recording'])
 check('simulated re-record starts a fresh take',not read()['controls']['page_record.transport.btn_save']['enabled'])
 click('page_record.transport.btn_record');wait(lambda r:not r['state']['recording'])
 discarded=read()['state']['discarded'];click('page_record.transport.btn_discard');wait(lambda r:r['state']['discarded']==discarded+1)
 check('simulated discard clears staged take',not read()['controls']['page_record.transport.btn_save']['enabled'])
 nav('hardware');check('incomplete calibration keeps Continue disabled',not read()['controls']['page_hardware.content.camera_a_panel.content.calib.actions.btn_continue']['enabled'])
 click('page_hardware.content.camera_a_panel.content.calib.actions.btn_restart');shot('simulated-calibration')
 nav('eval');click('page_eval.pbar.btn_stop');wait(lambda r:not r['controls']['page_eval.pbar.btn_stop']['enabled'])
 check('simulated evaluation Stop disables itself after stopping',not read()['controls']['page_eval.pbar.btn_stop']['enabled']);shot('simulated-evaluation-stopped')
else:
 cmd('window','k=resize','width=1440','height=900');nav('play')
 if read()['controls']['panel_tools']['visible']:click('panels_btn')
 click('panels_btn')
 key('Space');check('focused panel button does not also start playback',not read()['state']['playing'])
 if not read()['controls']['panel_tools']['visible']:click('panels_btn')
 click('layout_reset');wait(lambda r:r['state']['panel_layout']['order']==[0,1,2,3])
 before=rect('stage_3d');x,y,w,h=rect('error_panel.head');u,v,_,_=rect('robot_panel.head')
 drag(x+50,y+20,u+50,v+65)
 wait(lambda r:r['state']['panel_layout']['order']==[3,0,1,2])
 check('drag moves title and its actual 3D/plot content',rect('stage_3d')!=before and contained('stage_3d','robot_panel') and contained('stage_plot','error_panel'))
 x,y,w,h=rect('page_play.upper.grid');drag(x+(w-8)*.5+4,y+h*.25,x+w*.62,y+h*.25)
 wait(lambda r:r['state']['panel_layout']['split_x']>.58)
 check('vertical divider resizes panels',layout()['split_x']>.58)
 drag(x+w*.25,y+(h-8)*.5+4,x+w*.25,y+h*.62)
 wait(lambda r:r['state']['panel_layout']['split_y']>.58)
 check('horizontal divider resizes panels',layout()['split_y']>.58)
 click('error_panel.head.maximize');wait(lambda r:r['state']['panel_layout']['maximized']==3)
 check('maximize shows the intended plot',contained('stage_plot','error_panel') and not read()['controls']['robot_panel']['visible'])
 key('Escape');wait(lambda r:r['state']['panel_layout']['maximized'] is None)
 check('Escape restores panel grid',read()['controls']['robot_panel']['visible'])
 click('camera_a_panel.head.close');wait(lambda r:not r['state']['panel_layout']['visible'][1]);click('camera_a_toggle')
 wait(lambda r:r['state']['panel_layout']['visible'][1]);check('closed camera can be reopened with content intact',contained('stage_a','camera_a_panel'))
 saved=layout();click('appearance');wait(lambda r:r['state']['panel_layout']==saved)
 nav('library');nav('play');check('appearance and navigation retain custom layout',layout()==saved)
 shot('custom-layout')
 click('new_window');wait(lambda r:r['state']['monitor_open']);monitor=read()['state']['monitor_window']
 check('native Replay window opens with independent layout',read()['state']['monitor_layout']['order']==[0,1,2,3] and layout()==saved)
 # Use real monitor controls, proving routing doesn't accidentally hit the main view.
 click('monitor_play.play_btn',w=monitor);wait(lambda r:r['state']['playing'] and r['state']['time']>.3)
 click('monitor_play.play_btn',w=monitor);wait(lambda r:not r['state']['playing'])
 wait(lambda r:r['controls']['page_play.timeline.tl_body.controls.frame_lbl']['text']==r['controls']['monitor_play.timeline.tl_body.controls.frame_lbl']['text'])
 check('both windows share transport and frame position',read()['state']['time']>.3)
 if read()['state']['monitor_layout']['maximized']!=0:
  x,y,w,h=rect('monitor_play.upper.grid.robot_panel');cmd('click',f'x={x+w-66}',f'y={y+22}',w=monitor)
 wait(lambda r:r['state']['monitor_layout']['maximized']==0)
 check('extra window layout changes independently',layout()==saved)
 shot('native-replay-window',w=monitor)
 click('close_monitor',w=monitor);wait(lambda r:not r['state']['monitor_open'])
 check('closing extra window preserves main task',read()['state']['screen']=='Replay' and layout()==saved)
 click('new_window');wait(lambda r:r['state']['monitor_open']);check('extra window retains its layout after reopening',read()['state']['monitor_layout']['maximized']==0);click('close_monitor',w=monitor)
 check('extra window reopens after close',not read()['state']['monitor_open'])
 if read()['controls']['panel_tools']['visible']:click('panels_btn')
 click('page_play.upper.tree.tree_body.row_1');wait(lambda r:r['state']['episode']==1)
 key('Home');key('ArrowRight');check('single-frame stepping',abs(read()['state']['time']-1/30)<.001)
 x,y,w,h=rect('page_play.timeline.tl_body.tl');cmd('click',f'x={x+w*.3}',f'y={y+h*.5}')
 wait(lambda r:r['state']['time']>1);check('pointer scrubbing moves the shared playhead',1<read()['state']['time']<6)
 if not read()['controls']['page_play.upper.side']['visible']:click('details_btn')
 if read()['state']['tag']=='Some(Bad)':click('page_play.upper.side.side_body.btn_bad')
 click('page_play.upper.side.side_body.btn_bad');wait(lambda r:r['state']['tag']=='Some(Bad)')
 nav('library');nav('play')
 check('bad tag persists through navigation',read()['state']['tag']=='Some(Bad)')
 # Curation lives in a scrollable inspector; a larger scrub target leaves
 # less vertical space, so bring the action inside the viewport before input.
 x,y,w,h=rect('page_play.upper.side');cmd('mouse','k=scroll',f'x={x+w/2}',f'y={y+h/2}','dx=0','dy=500')
 click('page_play.upper.side.side_body.btn_del');wait(lambda r:r['state']['episode']!=1)
 key('KeyZ',logo=1);time.sleep(.5)
 if not read()['controls']['page_play.upper.tree']['visible']:click('details_btn')
 click('page_play.upper.tree.tree_body.row_1');wait(lambda r:r['state']['episode']==1)
 check('exclude and Undo recover the same episode and tag',read()['state']['tag']=='Some(Bad)')
 nav('library');click('page_library.left_col.header.btn_pull');click('hub_repo');cmd('key','t=sample/preview');key('Escape')
 wait(lambda r:not r['state']['hub_open']);check('cancel Hub form retains active episode',read()['state']['episode']==1)
 nav('record');check('missing rig keeps Record and Save disabled',not read()['controls']['page_record.transport.btn_record']['enabled'] and not read()['controls']['page_record.transport.btn_save']['enabled'])
 click('page_record.transport.btn_record');check('disabled Record cannot start capture',not read()['state']['recording']);shot('unconfigured-recording')
 nav('hardware');check('hardware has calibration controls and a recovery action',read()['controls']['page_hardware.hardware_reset']['visible'] and not read()['controls']['page_hardware.content.camera_a_panel.content.calib.actions.btn_continue']['enabled'])
 nav('eval');check('no policy runner keeps Stop disabled',not read()['controls']['page_eval.pbar.btn_stop']['enabled'])
 nav('play');x,y,w,h=rect('sidebar_resize');drag(x+w/2,y+h/2,236,y+h/2)
 check('navigation divider is resizable',abs(rect('sidebar')[2]-236)<2)
 key('Home');key('ArrowRight');time.sleep(.5)
 expected={'layout':layout(),'episode':read()['state']['episode'],'time':read()['state']['time'],'sidebar_width':rect('sidebar')[2],'tag':read()['state']['tag']}
 (a.out.parent/'expected-restart.json').write_text(json.dumps(expected,indent=2)+'\n')
 shot('before-restart')
print(f'{len(checks)} checks passed',flush=True)
