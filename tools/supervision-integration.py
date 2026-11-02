#!/usr/bin/env python3
"""Disposable foreground supervision, writer-safe wipe and service output."""
import hashlib,json,os,pathlib,shutil,signal,socket,subprocess,tempfile,time
ROOT=pathlib.Path(__file__).resolve().parents[1]
def port():
    with socket.socket() as sock:sock.bind(('127.0.0.1',0));return sock.getsockname()[1]
def main():
    report={'passed':False}
    with tempfile.TemporaryDirectory(prefix='lattice-supervisor-') as directory:
        base=pathlib.Path(directory);host=base/'host space%$quote"';ctl=base/'lattice';node=base/'lattice-node'
        for source,target in [(ROOT/'target/debug/lattice',ctl),(ROOT.parent/'lattice-node-rs/target/debug/lattice-node',node)]:
            shutil.copy2(source,target);report[target.name+'_sha256']=hashlib.sha256(target.read_bytes()).hexdigest()
        env=dict(os.environ,LATTICE_FORBID_DEFAULT_PEERS='1')
        def execute(*args,success=True):
            result=subprocess.run([str(ctl),'--root',str(host),*args],capture_output=True,text=True,env=env,timeout=45)
            assert (result.returncode==0)==success,result.stderr.replace(str(base),'<temporary>')
            return result.stdout
        def call(*args):return json.loads(execute(*args))
        def until(test):
            end=time.monotonic()+15
            while True:
                result=test()
                if result:return result
                assert time.monotonic()<end,'supervisor observation timed out'
                time.sleep(.05)
        listen,rpc=port(),port()
        while listen==rpc:rpc=port()
        call('init','--listen',str(listen),'--rpc-port',str(rpc),'--no-default-peers')
        identity=(host/'identity/Nexus.key').read_bytes();topology=(host/'lattice.json').read_bytes()
        record=host/'run/lattice-node.pid';log=base/'supervisor.log'
        with log.open('w') as output:
            supervisor=subprocess.Popen([str(ctl),'--root',str(host),'up','--foreground','--node-binary',str(node)],stdout=output,stderr=output,env=env)
            try:
                until(lambda:'supervisorReady' in log.read_text());first=json.loads(record.read_text())['process']['pid']
                execute('up','--foreground','--node-binary',str(node),success=False)
                execute('wipe',success=False)
                os.kill(first,signal.SIGKILL)
                until(lambda:record.exists() and json.loads(record.read_text())['process']['pid']!=first)
                until(lambda:log.read_text().count('supervisorReady')>=2)
                call('status');retained=json.loads(record.read_text())['process']['pid'];record.unlink()
                time.sleep(1.2);assert supervisor.poll() is None and not record.exists()
                supervisor.terminate();assert supervisor.wait(timeout=10)==0
                probe=subprocess.run(['ps','-o','stat=','-p',str(retained)],capture_output=True,text=True)
                assert probe.returncode!=0 or probe.stdout.strip().startswith('Z')
                assert not record.exists() and 'supervisorStopped' in log.read_text()
            finally:
                if supervisor.poll() is None:supervisor.terminate();supervisor.wait(timeout=40)
        # Missing PID cannot authorize deleting an actively locked database.
        detached=call('up','--node-binary',str(node));saved=record.read_bytes()
        execute('up','--foreground','--node-binary',str(base/'missing-node'),success=False)
        assert record.read_bytes()==saved and call('up','--node-binary',str(node))['pid']==detached['pid']
        nonexec=base/'nonexecutable-node';nonexec.write_text('not executable');nonexec.chmod(0o600)
        for invalid in [base,nonexec]:
            execute('up','--foreground','--node-binary',str(invalid),success=False)
            assert record.read_bytes()==saved and call('up','--node-binary',str(node))['pid']==detached['pid']
        call('status');record.unlink()
        try:execute('wipe',success=False)
        finally:record.write_bytes(saved);call('down')
        (host/'specs').mkdir(exist_ok=True);(host/'specs/keep').write_text('fixture')
        (host/'run/keep.json').write_text('{}')
        assert call('wipe')['wiped'] and not (host/'chains/Nexus').exists()
        assert (host/'identity/Nexus.key').read_bytes()==identity and (host/'lattice.json').read_bytes()==topology
        assert (host/'specs/keep').read_text()=='fixture' and (host/'run/keep.json').exists()
        assert not call('wipe')['wiped']
        outside=base/'outside';outside.mkdir();(outside/'keep').write_text('fixture')
        (host/'chains/Nexus').symlink_to(outside,target_is_directory=True)
        execute('wipe',success=False);assert (outside/'keep').exists();(host/'chains/Nexus').unlink()
        call('up','--node-binary',str(node));call('status');call('down')
        units=execute('emit-systemd','--binary','/bin/true','--node-binary','/bin/true')
        assert 'ExecStart=:' in units and '--foreground' in units and 'ExecStop=' not in units
        assert '%%' in units and '$' in units and '\\"' in units and 'lattice-mine.service' not in units
        configured=json.loads(topology);configured['mine']={'worker':'cpu'};(host/'lattice.json').write_text(json.dumps(configured))
        units=execute('emit-systemd','--binary','/bin/true','--node-binary','/bin/true')
        assert 'lattice-mine.service' in units and 'BindsTo=lattice-tree.service' in units
        out=ROOT/'target/systemd-fixtures';out.mkdir(exist_ok=True)
        # Public synthetic path keeps generated test fixtures free of local identity.
        units=units.replace(str(host).replace('\\','\\\\').replace('"','\\"').replace('%','%%'),'/var/lib/lattice test%%$quote\\"')
        for part in units.split('# ')[1:]:
            name,content=part.split('\n',1);(out/name).write_text(content)
        report['checks']=['foreground starts owned node','duplicate supervisor refused','wipe refuses supervisor','crash restarts node','PID-file loss retains process ownership','SIGTERM gracefully stops supervisor and child','initial supervisor failure preserves existing daemon','missing PID writer lock prevents wipe','wipe preserves identity topology specs journals','idempotent wipe','symlink storage refused','fresh startup after wipe','tree and conditional miner units quoted']
        report['passed']=True
    (ROOT/'target/supervision-integration.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report))
if __name__=='__main__':main()
