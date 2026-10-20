import json, subprocess, sys, tempfile, time
from pathlib import Path
sys.path.insert(0, str(Path('tools/release').resolve()))
import macos_dmg
package, output = map(Path, sys.argv[1:])
output.mkdir(exist_ok=False)
images=list(package.glob('*.dmg')); assert len(images)==1
script='''on run argv
set folderAlias to POSIX file (item 1 of argv) as alias
tell application "Finder"
activate
open folderAlias
set targetWindow to container window of folderAlias
set index of targetWindow to 1
delay 2
return bounds of targetWindow
end tell
end run
'''
close='''on run argv
set folderAlias to POSIX file (item 1 of argv) as alias
tell application "Finder" to close container window of folderAlias
end run
'''
with tempfile.TemporaryDirectory(prefix='nmt-dmg-preview-') as temp:
 mount=Path(temp)/'mount';mount.mkdir()
 with macos_dmg.attach(images[0],mount,False):
  try:
   result=subprocess.run(['osascript','-',str(mount)],input=script,text=True,capture_output=True,check=True,timeout=30)
   rect=list(map(int,result.stdout.strip().split(', ')));assert len(rect)==4
   x,y,right,bottom=rect;w,h=right-x,bottom-y;assert 0<w<2000 and 0<h<2000
   subprocess.run(['screencapture','-x','-R'+','.join(map(str,[x,y,w,h])),str(output/'finder-original.png')],check=True,timeout=15)
   (output/'capture.json').write_text(json.dumps({'bounds':rect,'observed_window_size':[w,h],'image':images[0].name,'app_executed':False},indent=2))
  finally:subprocess.run(['osascript','-',str(mount)],input=close,text=True,capture_output=True,check=True,timeout=30)
