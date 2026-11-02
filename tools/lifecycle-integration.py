#!/usr/bin/env python3
"""Real isolated lifecycle check: no public peers, wallets, or submitted work."""
import concurrent.futures, hashlib, json, os, pathlib, shutil, socket, subprocess, tempfile
ROOT = pathlib.Path(__file__).resolve().parents[1]
def port():
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        return sock.getsockname()[1]
def main():
    report = {'passed': False, 'scope': 'canonical Nexus, isolated localhost lifecycle'}
    with tempfile.TemporaryDirectory(prefix='lattice-lifecycle-') as directory:
        base = pathlib.Path(directory)
        ctl, node = base/'lattice', base/'lattice-node'
        for source, target in [(ROOT/'target/debug/lattice', ctl), (ROOT.parent/'lattice-node-rs/target/debug/lattice-node', node)]:
            shutil.copy2(source, target)
            report[target.name+'_sha256'] = hashlib.sha256(target.read_bytes()).hexdigest()
        host = base/'host'
        env = dict(os.environ, LATTICE_FORBID_DEFAULT_PEERS='1')
        def call(*args):
            result = subprocess.run([str(ctl), '--root', str(host), *args], capture_output=True, text=True, env=env, timeout=45)
            if result.returncode:
                raise RuntimeError('CLI failed: '+result.stderr.replace(str(base), '<temporary>'))
            return json.loads(result.stdout)
        listen, rpc = port(), port()
        while rpc == listen: rpc = port()
        host.mkdir()
        subprocess.run(['git','init','--quiet',str(host)], check=True, capture_output=True)
        first = call('init', '--listen', str(listen), '--rpc-port', str(rpc), '--no-default-peers')
        try:
            assert call('identity')['publicKey'] == first['publicKey']
            assert call('identity')['peerString'] == first['publicKey']+'@<this-host>:'+str(listen)
            assert call('identity')['listen'] == listen
            for ignored in ['identity/Nexus.key','chains/Nexus/.cookie','run/lattice-node.pid','log/lattice-node.log']:
                subprocess.run(['git','-C',str(host),'check-ignore','--quiet',ignored], check=True, capture_output=True)
            with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
                results = list(pool.map(lambda _: call('up'), range(2)))
            assert results[0]['pid'] == results[1]['pid']
            assert sum(row['started'] for row in results) == 1
            call('--chain','Nexus','status')
            cookie = (host/'chains/Nexus/.cookie').read_bytes()
            assert call('down')['stopped']
            assert not call('down')['stopped']
            restarted = call('up', '--node-binary', str(node))
            assert restarted['started'] and restarted['pid'] != results[0]['pid']
            assert (host/'chains/Nexus/.cookie').read_bytes() != cookie
            assert call('identity')['publicKey'] == first['publicKey']
            call('--chain','Nexus','status')
            topology = json.loads((host/'lattice.json').read_text())
            topology['minRelayFee'] = 2
            (host/'lattice.json').write_text(json.dumps(topology))
            changed = call('up', '--node-binary', str(node))
            assert changed['started'] and changed['pid'] != restarted['pid']
            key_path = host/'identity/Nexus.key'
            prior_seed = key_path.read_bytes()
            key_path.unlink()
            refused = subprocess.run([str(ctl),'--root',str(host),'up','--node-binary',str(node)], capture_output=True, env=env)
            assert refused.returncode != 0 and not key_path.exists()
            # Generate a second disposable identity through the production init path.
            other = base/'other'
            subprocess.run([str(ctl),'--root',str(other),'init','--no-default-peers'], check=True, capture_output=True, env=env)
            shutil.copy2(other/'identity/Nexus.key', key_path)
            assert key_path.read_bytes() != prior_seed
            replaced = call('up','--node-binary',str(node))
            assert replaced['started'] and replaced['pid'] != changed['pid']
            assert call('identity')['publicKey'] != first['publicKey']
            call('--chain','Nexus','status')
            peer_host = base/'peers'
            peers = [first['publicKey']+'@127.0.0.1:'+str(listen), first['publicKey']+'@[::1]:'+str(listen)]
            subprocess.run([str(ctl),'--root',str(peer_host),'init','--peer',peers[0],'--peer',peers[1]], check=True, capture_output=True, env=env)
            assert json.loads((peer_host/'lattice.json').read_text())['peers'] == peers
            invalid = base/'invalid'
            refused = subprocess.run([str(ctl),'--root',str(invalid),'init','--peer','invalid'], capture_output=True, env=env)
            assert refused.returncode and not invalid.exists()
            refused = subprocess.run([str(ctl),'--root',str(invalid),'init','--peer',peers[0],'--no-default-peers'], capture_output=True, env=env)
            assert refused.returncode and not invalid.exists()
            report['checks'] = ['repeatable explicit peers persist in order', 'invalid or conflicting peers leave host untouched', 'configured identity peer string', 'sibling daemon discovery starts actual daemon', 'canonical genesis init', 'private persistent identity', 'concurrent idempotent startup', 'authenticated status', 'idempotent stop', 'restart cookie rotation', 'changed topology restart', 'managed secrets ignored by Git', 'missing identity refuses startup', 'replacement identity restarts daemon']
            report['passed'] = True
        finally:
            call('down')
    out = ROOT/'target/lifecycle-integration.json'
    out.write_text(json.dumps(report, indent=2)+'\n')
    print(json.dumps(report))
if __name__ == '__main__': main()
