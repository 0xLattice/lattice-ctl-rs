#!/usr/bin/env python3
"""Actual CLI author/parent nodes, journal resume, and live child configuration."""
import argparse,hashlib,json,os,pathlib,shutil,socket,subprocess,tempfile,time
ROOT=pathlib.Path(__file__).resolve().parents[1]
def port():
    with socket.socket() as sock:sock.bind(('127.0.0.1',0));return sock.getsockname()[1]
def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--restart-author',action='store_true',help='Additional whole-host restart scenario; default retains author to test peer recovery')
    parser.add_argument('--keep',action='store_true',help='Retain disposable diagnostic directory')
    args=parser.parse_args()
    report={'passed':False,'scope':'two disposable localhost nodes and real child CLI'}
    with tempfile.TemporaryDirectory(prefix='lattice-child-cli-',delete=not args.keep) as directory:
        if args.keep: print('diagnostic directory: '+directory,flush=True)
        base=pathlib.Path(directory);ctl=base/'lattice';node=base/'lattice-node';children=[];handles=[]
        for source,target in [(ROOT/'target/debug/lattice',ctl),(ROOT.parent/'lattice-node-rs/target/debug/lattice-node',node)]:
            shutil.copy2(source,target);report[target.name+'_sha256']=hashlib.sha256(target.read_bytes()).hexdigest()
        env=dict(os.environ,LATTICE_FORBID_DEFAULT_PEERS='1')
        def execute(root,*args,success=True):
            result=subprocess.run([str(ctl),'--root',str(root),*args],capture_output=True,text=True,env=env,timeout=45)
            assert (result.returncode==0)==success,result.stderr.replace(str(base),'<temporary>')
            return result.stdout
        def call(root,*args):return json.loads(execute(root,*args))
        def until(test):
            end=time.monotonic()+30
            while True:
                result=test()
                if result:return result
                assert time.monotonic()<end,'timed out awaiting child observation'
                time.sleep(.1)
        parent,author=base/'parent',base/'author';ports=set()
        for root in [parent,author]:
            values=[]
            while len(values)<2:
                value=port()
                if value not in ports:ports.add(value);values.append(value)
            call(root,'init','--listen',str(values[0]),'--rpc-port',str(values[1]),'--no-default-peers')
        wallet=base/'fixture-wallet.json';owner=call(parent,'key','generate','--out',str(wallet))['address']
        parent_config=json.loads((parent/'lattice.json').read_text())
        parent_config['mine']={'worker':'cpu','batchSize':10000,'recipients':{'Nexus':owner}}
        (parent/'lattice.json').write_text(json.dumps(parent_config))
        author_config=json.loads((author/'lattice.json').read_text())
        author_config['peers']=[call(parent,'identity')['publicKey']+'@127.0.0.1:'+str(parent_config['listen'])]
        (author/'lattice.json').write_text(json.dumps(author_config))
        try:
            call(parent,'up','--node-binary',str(node));live=call(author,'up','--node-binary',str(node))['pid']
            # A rejected local preflight must leave topology and spec files unchanged.
            before=(author/'lattice.json').read_bytes()
            execute(author,'child','create','Nexus/Bad','--premine','1',success=False)
            assert (author/'lattice.json').read_bytes()==before and not list((author/'specs').glob('*Bad*'))
            created=call(author,'child','create','Nexus/Alpha','--block-time','1000','--reward','10','--premine','1','--premine-to',owner)
            assert created['appliedToRunningNode']
            assert call(author,'up','--node-binary',str(node))=={'running':True,'started':False,'pid':live}
            before=(author/'lattice.json').read_bytes();spec=(author/'specs/Nexus%2FAlpha.json').read_bytes()
            execute(author,'child','create','Nexus/Alpha',success=False)
            assert (author/'lattice.json').read_bytes()==before and (author/'specs/Nexus%2FAlpha.json').read_bytes()==spec
            execute(parent,'mine','run','--once')
            tip=call(parent,'status')['tipCID'];until(lambda:call(author,'status')['tipCID']==tip)
            journal=author/'run/child-Nexus%2FAlpha.json'
            def launch(label):
                handle=(base/(label+'.log')).open('w');handles.append(handle)
                proc=subprocess.Popen([str(ctl),'--root',str(author),'child','commit','Nexus/Alpha','--key',str(wallet),'--parent-rpc',f"http://127.0.0.1:{parent_config['rpc']}",'--parent-cookie',str(parent/'chains/Nexus/.cookie'),'--poll-ms','100','--timeout-seconds','30'],stdout=handle,stderr=handle,env=env)
                children.append(proc);return proc
            first=launch('first')
            until(lambda:journal.exists() and json.loads(journal.read_text()).get('accepted'))
            saved=json.loads(journal.read_text());assert saved['prepared']['body']['nonce']==0
            first.terminate();assert first.wait(timeout=5)==0
            assert not json.loads(journal.read_text())['done']
            cookie=(parent/'chains/Nexus/.cookie').read_bytes()
            call(parent,'down');call(parent,'up','--node-binary',str(node))
            assert (parent/'chains/Nexus/.cookie').read_bytes()!=cookie
            # Restart the author as well: automatic reconnect of a surviving
            # daemon after peer restart is a separately tracked node concern.
            if args.restart_author:
                call(author,'down');call(author,'up','--node-binary',str(node))
            second=launch('resume');time.sleep(.4)
            assert json.loads(journal.read_text())['prepared']==saved['prepared']
            execute(parent,'mine','run','--once')
            result=second.wait(timeout=35)
            assert result==0,'resumed author timed out' 
            completed=json.loads(journal.read_text());assert completed['done'] and completed['built']['blockCID']==saved['built']['blockCID']
            account=call(parent,'rpc','/api/state/account/'+owner);assert account['nonce']=='1'
            child_account=call(author,'--chain','Nexus/Alpha','rpc','/api/state/account/'+owner);assert child_account['balance']=='10'
            # Explicit hosting applies to a live parent without replacing the daemon.
            parent_pid=call(parent,'up','--node-binary',str(node))['pid']
            assert call(parent,'child','host','Nexus/Alpha')['appliedToRunningNode']
            assert call(parent,'up','--node-binary',str(node))['pid']==parent_pid
            assert call(author,'child','apply')['appliedToRunningNode']
            report['checks']=['invalid create preserves topology','create validates premine and applies live','duplicate create preserves artifacts','live reconciliation preserves daemon PID','remote author signs parent commitment','interrupted commit resumes exact signed transaction','parent restart with journal preserved','parent nonce increments once','child is weighed and receives premine','host existing child','explicit apply']
            report['author_restarted']=args.restart_author
            report['passed']=True
        finally:
            for proc in children:
                if proc.poll() is None:proc.terminate();proc.wait(timeout=5)
            for handle in handles:handle.close()
            call(author,'down');call(parent,'down')
    (ROOT/'target/child-integration.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report))
if __name__=='__main__':main()
