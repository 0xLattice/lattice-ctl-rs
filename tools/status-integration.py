#!/usr/bin/env python3
"""Owned tree health, no cookie, encoded child scope and bounded degradation."""
import hashlib,http.server,json,os,pathlib,shutil,socket,subprocess,tempfile,threading,time,urllib.parse
ROOT=pathlib.Path(__file__).resolve().parents[1]
def port():
    with socket.socket() as sock:sock.bind(('127.0.0.1',0));return sock.getsockname()[1]
def main():
    report={'passed':False}
    with tempfile.TemporaryDirectory(prefix='lattice-status-') as directory:
        base=pathlib.Path(directory);host=base/'host';ctl=base/'lattice';node=base/'lattice-node'
        for source,target in [(ROOT/'target/debug/lattice',ctl),(ROOT.parent/'lattice-node-rs/target/debug/lattice-node',node)]:
            shutil.copy2(source,target);report[target.name+'_sha256']=hashlib.sha256(target.read_bytes()).hexdigest()
        env=dict(os.environ,LATTICE_FORBID_DEFAULT_PEERS='1')
        def execute(*args,success=True):
            result=subprocess.run([str(ctl),'--root',str(host),*args],capture_output=True,text=True,env=env,timeout=15)
            assert (result.returncode==0)==success,result.stderr.replace(str(base),'<temporary>')
            return result.stdout
        def call(*args):return json.loads(execute(*args))
        listen,rpc=port(),port()
        while listen==rpc:rpc=port()
        call('init','--listen',str(listen),'--rpc-port',str(rpc),'--no-default-peers')
        assert call('status','--json')['process']=='down'
        assert execute('status')=='Nexus: down\n'
        try:
            call('up','--node-binary',str(node));call('child','host','Nexus/Waiting')
            explicit=call('--chain','Nexus','status');assert explicit['chainPath']==['Nexus'] and 'revision' in explicit
            cookie=host/'chains/Nexus/.cookie';saved=cookie.read_bytes();cookie.unlink()
            try:
                tree=call('status','--json');assert tree['process']=='running'
                assert [row['chainPath'] for row in tree['chains']]==[['Nexus'],['Nexus','Waiting']]
                assert all(row['state']=='available' for row in tree['chains'])
                assert tree['chains'][0]['health']['phase']=='active' and tree['chains'][1]['health']['phase']=='awaitingGenesis'
                table=execute('status');assert 'Nexus: active' in table and 'Nexus/Waiting: awaitingGenesis' in table
                execute('--chain','Nexus','status',success=False)
            finally:cookie.write_bytes(saved);cookie.chmod(0o600)
            # Keep the owned real daemon running while a localhost health server
            # degrades one path. Requests must carry no cookie/auth header.
            observed=[]
            class Handler(http.server.BaseHTTPRequestHandler):
                def log_message(self,*args):pass
                def do_GET(self):
                    query=urllib.parse.parse_qs(urllib.parse.urlsplit(self.path).query)
                    path=query.get('chainPath',['Nexus'])[0];observed.append((path,self.headers.get('Authorization')))
                    if path!='Nexus':time.sleep(6)
                    payload={'phase':'active','chainPath':path.split('/'),'nexusGenesisCID':'fixture','height':'18446744073709551615','tipCID':'fixture','mempoolCount':0,'mempoolBytes':0}
                    body=json.dumps(payload).encode()
                    try:
                        self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
                    except (BrokenPipeError,ConnectionResetError):pass
            server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Handler)
            threading.Thread(target=server.serve_forever,daemon=True).start()
            start=time.monotonic();degraded=call('--rpc',f'http://127.0.0.1:{server.server_port}','status','--json');elapsed=time.monotonic()-start
            assert elapsed<7 and degraded['chains'][0]['state']=='available' and degraded['chains'][1]['state']=='unreachable'
            assert degraded['chains'][0]['health']['height']=='18446744073709551615'
            assert set(path for path,_ in observed)=={'Nexus','Nexus/Waiting'} and all(auth is None for _,auth in observed)
            server.shutdown();server.server_close()
            start=time.monotonic();unreachable=call('--rpc',f'http://127.0.0.1:{port()}','status','--json')
            assert time.monotonic()-start<7 and all(row['state']=='unreachable' for row in unreachable['chains'])
            # A stale PID never presents an unrelated process as an owned daemon.
            pid=host/'run/lattice-node.pid';record=pid.read_bytes();value=json.loads(record);value['process']['birth']+='-stale';pid.write_text(json.dumps(value))
            try:assert call('status','--json')['process']=='down'
            finally:pid.write_bytes(record)
            report['checks']=['default table across committed tree','down without cookie','real root and awaiting child health','missing cookie does not block health','explicit chain retains authenticated JSON status','encoded child paths','no Authorization header','partial health with global five-second deadline','wide integer presentation','bounded unreachable RPC','stale ownership refuses running claim']
            report['passed']=True
        finally:call('down')
    (ROOT/'target/status-integration.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report))
if __name__=='__main__':main()
