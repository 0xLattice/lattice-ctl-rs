#!/usr/bin/env python3
"""Real apply-before-record failure and mock apply-before-timeout recovery."""
import hashlib,http.server,json,os,pathlib,shutil,socket,subprocess,tempfile,threading,time
ROOT=pathlib.Path(__file__).resolve().parents[1]
def port():
    with socket.socket() as sock:sock.bind(('127.0.0.1',0));return sock.getsockname()[1]
def main():
    report={'passed':False}
    with tempfile.TemporaryDirectory(prefix='lattice-child-recovery-') as directory:
        base=pathlib.Path(directory);host=base/'host';ctl=base/'lattice';node=base/'lattice-node'
        for source,target in [(ROOT/'target/debug/lattice',ctl),(ROOT.parent/'lattice-node-rs/target/debug/lattice-node',node)]:
            shutil.copy2(source,target);report[target.name+'_sha256']=hashlib.sha256(target.read_bytes()).hexdigest()
        env=dict(os.environ,LATTICE_FORBID_DEFAULT_PEERS='1')
        def execute(*args,success=True):
            result=subprocess.run([str(ctl),'--root',str(host),*args],capture_output=True,text=True,env=env,timeout=45)
            assert (result.returncode==0)==success,result.stderr.replace(str(base),'<temporary>')
            return result
        def call(*args):return json.loads(execute(*args).stdout)
        listen,rpc=port(),port()
        while listen==rpc:rpc=port()
        call('init','--listen',str(listen),'--rpc-port',str(rpc),'--no-default-peers')
        call('up','--node-binary',str(node))
        pending=host/'run/pending-topology.json';config=host/'run/node-config.json';record=host/'run/lattice-node.pid'
        try:
            original=(host/'lattice.json').read_bytes();saved=config.read_bytes();config.unlink();config.mkdir()
            execute('child','create','Nexus/Recorded',success=False)
            assert pending.exists() and (host/'specs/Nexus%2FRecorded.json').exists()
            assert (host/'lattice.json').read_bytes()==original
            assert 'Recorded' in json.dumps(call('rpc','/hosted-chains'))
            config.rmdir();config.write_bytes(saved)
            assert call('child','apply')['appliedToRunningNode']
            assert not pending.exists() and 'Nexus/Recorded' in json.loads((host/'lattice.json').read_text())['hostedChains']
            # A localhost test peer records application but withholds the response
            # beyond LocalRpc's real 30-second deadline. No production failpoint.
            state={'requests':0,'applied':[]};mutex=threading.Lock()
            class Handler(http.server.BaseHTTPRequestHandler):
                def log_message(self,*args):pass
                def do_POST(self):
                    body=json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                    with mutex:
                        state['requests']+=1;first=state['requests']==1;state['applied']=body['hostChain']
                    if first:time.sleep(31)
                    try:
                        self.send_response(200);self.send_header('Content-Type','application/json');self.end_headers();self.wfile.write(b'{}')
                    except (BrokenPipeError,ConnectionResetError):pass
            server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Handler)
            threading.Thread(target=server.serve_forever,daemon=True).start();mockport=server.server_port
            # Redirect only the disposable CLI's endpoint records to the mock.
            topology=json.loads((host/'lattice.json').read_text());topology['rpc']=mockport;(host/'lattice.json').write_text(json.dumps(topology))
            value=json.loads(record.read_text());value['config']['rpc']=f'127.0.0.1:{mockport}';record.write_text(json.dumps(value))
            before=(host/'lattice.json').read_bytes()
            result=execute('child','create','Nexus/Timeout',success=False)
            assert 'timed out' in result.stderr or 'request' in result.stderr
            assert pending.exists() and (host/'specs/Nexus%2FTimeout.json').exists()
            assert (host/'lattice.json').read_bytes()==before and any('Nexus/Timeout=' in row for row in state['applied'])
            assert call('child','apply')['appliedToRunningNode']
            assert state['requests']==2 and not pending.exists()
            assert 'Nexus/Timeout' in json.loads((host/'lattice.json').read_text())['hostedChains']
            server.shutdown();server.server_close()
            # Stage another valid desired topology and let up complete it by restart.
            desired=json.loads((host/'lattice.json').read_text());desired['rpc']=rpc;desired['hostedChains'].append('Nexus/Restart')
            pending.write_text(json.dumps({'version':1,'operation':'apply-hosted-chains','topology':desired}))
            assert call('up','--node-binary',str(node))['started']
            assert not pending.exists() and 'Restart' in json.dumps(call('rpc','/hosted-chains'))
            report['checks']=['real apply succeeds before record-file failure','artifacts and durable desired topology survive failure','child apply completes record recovery','mock application before actual HTTP timeout','timeout preserves artifacts and intent','retry converges without recreate','up consumes pending desired topology']
            report['passed']=True
        finally:call('down')
    (ROOT/'target/child-recovery-integration.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report))
if __name__=='__main__':main()
