#!/usr/bin/env python3
"""Disposable local node + actual shared CPU/process worker lifecycle."""
import hashlib, json, os, pathlib, shutil, socket, subprocess, tempfile, time
ROOT=pathlib.Path(__file__).resolve().parents[1]
def port():
    with socket.socket() as sock:
        sock.bind(('127.0.0.1',0)); return sock.getsockname()[1]
def main():
    report={'passed':False,'scope':'disposable localhost mining lifecycle; no real wallet'}
    with tempfile.TemporaryDirectory(prefix='lattice-mine-') as directory:
        base=pathlib.Path(directory);host=base/'host';ctl=base/'lattice';node=base/'lattice-node'
        for source,target in [(ROOT/'target/debug/lattice',ctl),(ROOT.parent/'lattice-node-rs/target/debug/lattice-node',node)]:
            shutil.copy2(source,target);report[target.name+'_sha256']=hashlib.sha256(target.read_bytes()).hexdigest()
        env=dict(os.environ,LATTICE_FORBID_DEFAULT_PEERS='1')
        def execute(*args, success=True):
            result=subprocess.run([str(ctl),'--root',str(host),*args],env=env,text=True,capture_output=True,timeout=45)
            assert (result.returncode==0)==success, result.stderr.replace(str(base),'<temporary>')
            return result.stdout
        def call(*args): return json.loads(execute(*args))
        def configure(mine):
            path=host/'lattice.json';topology=json.loads(path.read_text());topology['mine']=mine;path.write_text(json.dumps(topology))
        def events():
            path=host/'log/mine.log'
            if not path.exists(): return []
            rows=[]
            for line in path.read_text().splitlines():
                try: rows.append(json.loads(line))
                except ValueError: pass
            return rows
        def accepted():return sum(row.get('accepted',False) for row in events())
        def until(test):
            deadline=time.monotonic()+20
            while not test():
                assert time.monotonic()<deadline,'timed out awaiting mining observation'
                time.sleep(.05)
        listen,rpc=port(),port()
        while listen==rpc: rpc=port()
        call('init','--listen',str(listen),'--rpc-port',str(rpc),'--no-default-peers')
        wallet=call('key','generate','--out',str(base/'fixture-wallet.json'))
        plan={'worker':'cpu','workers':2,'batchSize':10000,'recipients':{'Nexus':wallet['address']},'minWork':{'Nexus':'1'},'minBlockIntervalSeconds':1,'templateTimeoutSeconds':1}
        configure(plan)
        try:
            call('up','--node-binary',str(node))
            first=call('mine','start')
            execute('mine','start',success=False)
            until(lambda:accepted()>=1)
            state=call('rpc','/api/state/account/'+wallet['address']);assert int(state['balance'])>=1048576
            status=call('mine','status');assert status['running']==first['pid'] and status['plan']['recipients'][0]['address']==wallet['address']
            before=accepted();cookie=(host/'chains/Nexus/.cookie').read_bytes()
            call('down');call('up','--node-binary',str(node))
            assert (host/'chains/Nexus/.cookie').read_bytes()!=cookie
            until(lambda:accepted()>before)
            started=time.monotonic();assert call('mine','stop')['stopped'];assert time.monotonic()-started<5
            assert call('mine','status')['running'] is None
            assert events()[-1]['event']=='mineStopped'
            call('down');call('mine','start')
            until(lambda:events()[-1].get('event')=='notMining')
            started=time.monotonic();call('mine','stop');assert time.monotonic()-started<5
            call('up','--node-binary',str(node))
            configure(dict(plan,batchSize=2000000000,minWork={'Nexus':'2^255'}))
            call('mine','start');time.sleep(.2)
            started=time.monotonic();call('mine','stop');assert time.monotonic()-started<5
            # A deliberately wedged disposable process worker exercises cancellation.
            worker=base/'worker.sh'
            worker.write_text('#!/bin/sh\necho $$ > "$0.pid"\nsleep 60 &\necho $! > "$0.child"\nwait\n');worker.chmod(0o700)
            configure(dict(plan,worker=str(worker),workers=1))
            call('mine','start');until(lambda:worker.with_suffix('.sh.child').exists())
            worker_pid=int(worker.with_suffix('.sh.pid').read_text());child_pid=int(worker.with_suffix('.sh.child').read_text())
            started=time.monotonic();call('mine','stop');assert time.monotonic()-started<5
            for pid in [worker_pid,child_pid]:
                def gone():
                    result=subprocess.run(['ps','-o','stat=','-p',str(pid)],capture_output=True,text=True)
                    return result.returncode!=0 or result.stdout.strip().startswith('Z')
                until(gone)
            configure(plan)
            once=[json.loads(line) for line in execute('mine','run','--once').splitlines()]
            assert any(row.get('accepted') for row in once)
            report['checks']=['declared recipient receives real CPU-mined reward','minimum work plan','duplicate miner refusal','node restart cookie recovery','graceful stop during pacing','offline-node cancellation','busy CPU worker cancellation','process worker group cleanup','foreground once']
            report['passed']=True
        finally:
            call('mine','stop');call('down')
    (ROOT/'target/mining-integration.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report))
if __name__=='__main__':main()
