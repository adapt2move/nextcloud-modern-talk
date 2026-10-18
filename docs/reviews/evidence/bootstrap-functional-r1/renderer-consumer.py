#!/usr/bin/env python3
"""BAR1-02 reproduction in a temporary copy, never in the candidate."""
import difflib
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

work=Path(sys.argv[1]); fixture=work/'architecture-fixture'; evidence=work/'evidence'
path=fixture/'apps/bootstrap/src/presentation.rs'; original=path.read_text()
env=dict(os.environ,CARGO_HOME='/opt/nmt-bootstrap-toolchain/cargo',RUSTUP_HOME='/opt/nmt-bootstrap-toolchain/rustup',
         CARGO_TARGET_DIR=str(work/'renderer-consumer-target'),PYTHONDONTWRITEBYTECODE='1')
env['PATH']='/opt/nmt-bootstrap-toolchain/cargo/bin:'+env['PATH']
def run(argv):
    p=subprocess.run(argv,cwd=fixture,env=env,capture_output=True,text=True,timeout=120)
    return {'argv':argv,'exit_code':p.returncode,'stdout':p.stdout.replace(str(work),'[review-temp]'),'stderr':p.stderr.replace(str(work),'[review-temp]')}

body='''
pub fn describe(backend: &mut dyn DesktopPort) {
    if let Some(snapshot) = backend.take_snapshot() {
        let _label = match snapshot.persistence {
            PersistenceState::Unconfigured => "unconfigured",
            PersistenceState::Stored(_) => "stored",
            PersistenceState::SessionOnly => "session only",
            PersistenceState::SessionOnlyCleanupPending => "cleanup pending",
            PersistenceState::RestartSuppressionUnconfirmed => "suppression unconfirmed",
            PersistenceState::RecoveryRequired => "recovery required",
        };
        for row in snapshot.conversations.as_slice() {
            let _available = match row.can_join {
                Decision::Allowed => true,
                Decision::Denied | Decision::Unknown | Decision::Stale => false,
            };
        }
    }
}
#[allow(dead_code)]
pub fn typed_intents(account: AccountId, call: CallScope, conversation: ConversationId) -> [UserIntent; 3] {
    [UserIntent::Logout(account), UserIntent::SelectConversation(conversation),
     UserIntent::AcceptDirect { call, media: MediaIntent::MicrophoneOffCameraOff }]
}
'''
permitted='use talk_core::ui::{AccountId, CallScope, ConversationId, Decision, DesktopPort, MediaIntent, PersistenceState, UserIntent};\n'+body
defining='''use talk_core::ui::{DesktopPort, MediaIntent, UserIntent};
use talk_core::policy::Decision;
use talk_core::identity::PersistenceState;
use talk_core::ids::{AccountId, CallScope, ConversationId};
'''+body
baseline=run(['python3','tools/check_architecture.py'])
assert baseline['exit_code']==0
results=[]
try:
    for name,text in [('permitted-ui-imports',permitted),('defining-module-imports',defining)]:
        path.write_text(text)
        lock_before=hashlib.sha256((fixture/'Cargo.lock').read_bytes()).hexdigest()
        checker=run(['python3','tools/check_architecture.py'])
        compiler=run(['cargo','check','--workspace','--all-targets','--locked'])
        assert hashlib.sha256((fixture/'Cargo.lock').read_bytes()).hexdigest()==lock_before
        results.append({'case':name,'checker':checker,'compiler':compiler,'fixture_diff':''.join(difflib.unified_diff(original.splitlines(True),text.splitlines(True),fromfile='candidate/presentation.rs',tofile=name+'/presentation.rs')),
                        'source_sha256':hashlib.sha256(text.encode()).hexdigest()})
        path.write_text(original)
        restored=run(['python3','tools/check_architecture.py'])
        assert restored['exit_code']==0
        results[-1]['restored_checker']=restored
finally:
    path.write_text(original)
result={'related_finding':'BAR1-02','candidate':'0b6e94db7faa4bee3d2ae88a5d2da2a5567e05be','baseline':baseline,'cases':results,
        'note':'Expected usable positive consumer is impossible through permitted UI exports. This is a compile/guard fixture, not GPUI/native execution.'}
result['defect_reproduced']=(results[0]['checker']['exit_code']==0 and results[0]['compiler']['exit_code']!=0 and
    results[0]['compiler']['stderr'].count('error[E0603]')==5 and results[1]['checker']['exit_code']==1 and results[1]['compiler']['exit_code']==0)
(evidence/'renderer-consumer.json').write_text(json.dumps(result,indent=2,sort_keys=True)+'\n')
print(json.dumps(result,indent=2),flush=True)
