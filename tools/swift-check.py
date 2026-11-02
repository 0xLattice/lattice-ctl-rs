#!/usr/bin/env python3
"""Compare actual pinned Swift keys and signed action content, not a mock CLI."""
import hashlib,json,pathlib,subprocess,tempfile
ROOT=pathlib.Path(__file__).resolve().parents[1];CACHE=pathlib.Path('/tmp/lattice-genesis-oracle');REF=ROOT.parent/'Lattice';NODE=ROOT.parent/'lattice-node'
PIN='dda2d852429a27be6043599b63bde81530cd9514'
assert subprocess.check_output(['git','rev-parse','HEAD'],cwd=REF,text=True).strip()==PIN
assert subprocess.check_output(['git','rev-parse','HEAD'],cwd=NODE,text=True).strip()=='3df9e0a786c7b103a3da88d1c3881efc81706255'
for path in (REF/'Sources').rglob('*.swift'):assert path.read_bytes()==(CACHE/path.relative_to(REF)).read_bytes()
assert hashlib.sha256((CACHE/'Package.resolved').read_bytes()).hexdigest()=='958f69bd3ec734a372e8cd94e3d09d593674a1025979a2abe2b45a6f08cfc9a6'
target=CACHE/'Sources/CtlSwiftOracle';target.mkdir(exist_ok=True)
(target/'main.swift').write_bytes((ROOT/'tests/swift-oracle.swift').read_bytes())
p=CACHE/'Package.swift';text=p.read_text()
if 'name: "CtlSwiftOracle"' not in text:
 text=text.replace('    targets: [\n','    targets: [\n        .executableTarget(name: "CtlSwiftOracle", dependencies: ["Lattice", .product(name:"cashew",package:"cashew"), .product(name:"Crypto",package:"swift-crypto"), .product(name:"Multikey",package:"Multikey")]),\n',1);p.write_text(text)
with tempfile.TemporaryDirectory() as tmp:
 path=pathlib.Path(tmp)/'samples.json'
 with path.open('w') as out:subprocess.run(['cargo','run','--quiet','--locked','--example','swift_samples'],cwd=ROOT,stdout=out,check=True)
 rows=json.loads(path.read_text())
 expected=json.loads(subprocess.check_output(['swift','run','--package-path',str(CACHE),'--force-resolved-versions','CtlSwiftOracle',str(path)]))
 assert len(rows)==len(expected)==80
 for actual,want in zip(rows,expected):
  for key in ["address","publicKey","bodyCID","preimage"]:assert actual[key]==want[key],(actual,key,want[key])
  assert want["rustSignatureAccepted"]=="true"
  actual['swift']=want
 corpus=json.dumps(rows,indent=2)+'\n';(ROOT/'tests/swift-vectors.json').write_text(corpus)
 evidence={'passed':True,'cases':len(rows),'lattice_revision':PIN,'node_revision':'3df9e0a786c7b103a3da88d1c3881efc81706255','corpus_sha256':hashlib.sha256(corpus.encode()).hexdigest(),'source_sha256':{str(p.relative_to(ROOT)):hashlib.sha256(p.read_bytes()).hexdigest() for p in [ROOT/'src/transaction.rs',ROOT/'examples/swift_samples.rs',ROOT/'tests/swift-oracle.swift',ROOT/'tools/swift-check.py']},'method':'Actual unchanged Swift Lattice/Crypto/Multikey; fixed seeds and five action shapes mirror LatticeCtl/Tx.swift assembly. Compare key/address/bodyCID/preimage exactly; cross-verify signatures and bind each transaction CID to its actual signature. Swift native signatures may be randomized. Does not execute the full Swift CLI.'}
 subprocess.run(['cargo','test','--locked','--test','swift_vectors'],cwd=ROOT,check=True)
 (ROOT/'tests/swift-evidence.json').write_text(json.dumps(evidence,indent=2)+'\n');print('80 actual Swift signed-content comparisons passed')
