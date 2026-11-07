"""Bounded synthetic released-server workload for the native diagnostic runner."""
import base64
import hashlib
import json
from pathlib import Path
import urllib.request


def seed(lab, rooms):
    # NextCloud limits share creation to 20 per 600 seconds. This owned fixture
    # needs 50 shares, all outside the timed workload. The global protection
    # switch only stops new accounting; existing attempts still cause 429s.
    # Use the server's action-specific override during seeding, then remove it
    # and verify restoration before timing, also on preparation failure.
    key = 'ratelimit_overwrite'
    previous = lab.occ('config:system:get', key, '--output=json', '--default-value=unset').strip()
    if json.loads(previous) != 'unset':
        raise RuntimeError('Disposable fixture already has rate overrides; refusing to replace them')
    try:
        action = 'files_sharing.shareapi.createshare'
        lab.occ('config:system:set', key, action, 'user', 'limit', '--type=integer', '--value=1000')
        lab.occ('config:system:set', key, action, 'user', 'period', '--type=integer', '--value=600')
        workload = _seed(lab, rooms)
    finally:
        lab.occ('config:system:delete', key)
        restored = lab.occ('config:system:get', key, '--output=json', '--default-value=unset').strip()
        if restored != previous:
            raise RuntimeError('Disposable fixture rate-limit setting was not restored')
    workload['preparation'] = 'Share rate limit suspended only for synthetic seeding; exact prior setting restored and verified before timing'
    return workload


def _seed(lab, rooms):
    """Exactly 200 rooms; 1,000+ messages with images, polls, voice and threads.

    Preparation is outside every timed interval. The account and files belong
    exclusively to the disposable lab. No production data or external URLs.
    """
    listing=lab.request('lab_alice','GET','/ocs/v2.php/apps/spreed/api/v4/room',name='perf.list')
    keep={room['token'] for room in rooms}
    for room in listing:
        if room['token'] not in keep and room.get('name','').startswith('Native performance room'):
            lab.request('lab_alice','DELETE',f"/ocs/v2.php/apps/spreed/api/v4/room/{room['token']}",expected=(200,404),name='perf.remove-extra')
    listing=lab.request('lab_alice','GET','/ocs/v2.php/apps/spreed/api/v4/room',name='perf.count-base')
    if len(listing)>200: raise RuntimeError('Owned fixture already exceeds the fixed 200-room workload')
    for index in range(200-len(listing)):
        lab.request('lab_alice','POST','/ocs/v2.php/apps/spreed/api/v4/room',body={'roomType':2,'roomName':f'Native performance room {index:03}'},expected=(201,),name='perf.room')
    token=rooms[0]['token']
    chat=f'/ocs/v2.php/apps/spreed/api/v1/chat/{token}'
    for index in range(900):
        lab.request('lab_bob','POST',chat,body={'message':f'Synthetic performance message {index:04}: **bounded Markdown**, `code`, list\n- task\n- detail'},expected=(201,),name='perf.text')
    root=Path(__file__).resolve().parents[2]
    photo=(root/'apps/desktop/assets/fixtures/content-photo.png').read_bytes()
    wave=(root/'apps/desktop/assets/sounds/message-notification.wav').read_bytes()
    files=[]
    for index in range(25):
        for name,data,mime in [(f'native-perf-{index}.png',photo,'image/png'),(f'native-perf-{index}.wav',wave,'audio/wav')]:
            path='/'+name;files.append(path)
            credentials=base64.b64encode(f"lab_alice:{lab.passwords['lab_alice']}".encode()).decode()
            request=urllib.request.Request(lab.base+'/remote.php/dav/files/lab_alice'+path,data=data,method='PUT',headers={'Authorization':'Basic '+credentials,'Host':lab.http_host,'Content-Type':mime,'OCS-APIRequest':'true'})
            with lab.clients['lab_alice'].open(request,timeout=20) as response:
                if response.status not in (201,204): raise RuntimeError('Synthetic DAV upload failed')
            lab.request('lab_alice','POST','/ocs/v2.php/apps/files_sharing/api/v1/shares',body={'shareType':10,'shareWith':token,'path':path,'referenceId':f'native-perf-{index}-{mime}','talkMetaData':json.dumps({'messageType':'voice-message'} if mime=='audio/wav' else {})},expected=(200,),name='perf.file-share')
        lab.request('lab_alice','POST',f'/ocs/v2.php/apps/spreed/api/v1/poll/{token}',body={'question':f'Synthetic poll {index}','options':['Option A','Option B'],'resultMode':0,'maxVotes':1},expected=(200,201),name='perf.poll')
        parent=lab.request('lab_bob','POST',chat,body={'message':f'Synthetic thread root {index}','threadTitle':f'Synthetic thread {index}'},expected=(201,),name='perf.thread-root')
        lab.request('lab_alice','POST',chat,body={'message':f'Synthetic thread reply {index}','threadId':parent['id']},expected=(201,),name='perf.thread-reply')
    # Validate the real corpus, rather than claiming success from enqueue counts.
    ids=set();counts={'images':0,'voice':0,'polls':0,'threads':0};before=0
    for _ in range(20):
        page=lab.request('lab_alice','GET',chat,query={'lookIntoFuture':0,'limit':100,'lastKnownMessageId':before},name='perf.validate-history')
        if not page: break
        for message in page:
            number=int(message['id'])
            if number in ids: raise RuntimeError('Duplicate server history ID')
            ids.add(number)
            if message.get('messageType')=='voice-message': counts['voice']+=1
            if message.get('threadId',0) or message.get('threadTitle'): counts['threads']+=1
            for value in message.get('messageParameters',{}).values():
                if value.get('type')=='talk-poll': counts['polls']+=1
                if value.get('type')=='file' and value.get('mimetype','').startswith('image/'):counts['images']+=1
        before=min(int(item['id']) for item in page)
        if len(page)<100: break
    listing=lab.request('lab_alice','GET','/ocs/v2.php/apps/spreed/api/v4/room',name='perf.validate-rooms')
    if len(listing)!=200 or len(ids)<1000 or not all(counts.values()):
        raise RuntimeError(f'Synthetic workload validation failed: {len(listing)} rooms, {len(ids)} messages, {counts}')
    return {'rooms':len(listing),'server_messages':len(ids),'mixed':counts,'loaded_page_limit':100,'image_sha256':hashlib.sha256(photo).hexdigest(),'wave_sha256':hashlib.sha256(wave).hexdigest(),'generator':'native_workload.py, sequential 900 Markdown + 25 image/voice/poll/thread groups','files':files}


def cleanup(lab,workload):
    for path in workload['files']:
        credentials=base64.b64encode(f"lab_alice:{lab.passwords['lab_alice']}".encode()).decode()
        request=urllib.request.Request(lab.base+'/remote.php/dav/files/lab_alice'+path,method='DELETE',headers={'Authorization':'Basic '+credentials,'Host':lab.http_host})
        with lab.clients['lab_alice'].open(request,timeout=20) as response:
            if response.status!=204:raise RuntimeError('Synthetic DAV cleanup failed')
