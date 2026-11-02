#!/usr/bin/env python3
"""Actual CLI signed payment, pending-nonce refusal and cookie rotation on localhost."""
import hashlib,importlib.util,json,pathlib,shutil,subprocess,tempfile,time
ROOT=pathlib.Path(__file__).resolve().parents[1];NODE=ROOT.parent/'lattice-node-rs';WORK=ROOT.parent/'lattice-rs'
spec=importlib.util.spec_from_file_location('child',NODE/'tools/child-node-integration.py');shared=importlib.util.module_from_spec(spec);spec.loader.exec_module(shared)
OUT=ROOT/'target/node-integration';OUT.mkdir(parents=True,exist_ok=True)
report={'passed':False,'started':time.time(),'scope':'fixture-only localhost daemon and real CLI'}
def save():(OUT/'report.json').write_text(json.dumps(report,indent=2)+'\n')
def snapshot():
 result=shared.snapshot()
 for repo in [ROOT,ROOT.parent/'lattice-sdk-rs',ROOT.parent/'lattice-light-client-rs',ROOT.parent/'multikey-rs']:
  for path in list(repo.glob('Cargo.*'))+list(repo.glob('src/**/*.rs'))+list(repo.glob('tools/node-integration.py')):
   result[str(path.relative_to(ROOT.parent))]=hashlib.sha256(path.read_bytes()).hexdigest()
 return result
save();before=snapshot();processes=[]
try:
 with (OUT/'build.log').open('w') as log:
  for cwd,args in [(ROOT,['--bin','lattice']),(NODE,['--bin','lattice-node']),(WORK,['-p','lattice-validation','--example','mixed_node_signer','--example','nexus_genesis_volumes'])]:subprocess.run(['cargo','build','--locked']+args,cwd=cwd,stdout=log,stderr=subprocess.STDOUT,check=True)
 with tempfile.TemporaryDirectory(prefix='lattice-cli-') as tmp:
  directory=pathlib.Path(tmp)
  binary=pathlib.Path(shutil.copy2(NODE/'target/debug/lattice-node',directory/'node'))
  ctl=pathlib.Path(shutil.copy2(ROOT/'target/debug/lattice',directory/'lattice'))
  signer=pathlib.Path(shutil.copy2(WORK/'target/debug/examples/mixed_node_signer',directory/'signer'))
  report['binary_sha256']={p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in [binary,ctl,signer]}
  with (directory/'bootstrap.json').open('w') as stream:subprocess.run([str(WORK/'target/debug/examples/nexus_genesis_volumes')],stdout=stream,check=True)
  identity=json.loads(subprocess.check_output([str(signer)],text=True));identity['privateKey']='4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb'
  key=directory/'fixture-key.json';key.write_text(json.dumps(identity));key.chmod(0o600)
  node=shared.Node('node-data',directory,binary,OUT,processes);node.start()
  def cli(args,success=True):
   result=subprocess.run([str(ctl),'--rpc',node.base,'--cookie',str(node.cookie)]+args,text=True,capture_output=True)
   assert (result.returncode==0)==success,(args,result.stdout,result.stderr)
   return json.loads(result.stdout) if success else result.stderr
  def mine():
   code,template=node.rpc('/mining/templates',{'recipients':[{'chainPath':['Nexus'],'address':identity['address']}]});assert code==200,(code,template)
   path=directory/'template.json';path.write_text(json.dumps(template));work=json.loads(subprocess.check_output([str(signer),'--mine',str(path)],text=True))
   digest=hashlib.sha256(bytes(work['prefix'])+work['nonce'].to_bytes(8,'big')).digest();assert digest==bytes(work['hash']) and int.from_bytes(digest,'big')<=int(template['searchTarget'],16)
   code,result=node.rpc('/mining/work',{'workID':template['workID'],'nonce':work['nonce']});assert code==200 and result['accepted'],(code,result)
   return result
  report['initial_status']=cli(['--chain','Nexus','status']);mine()
  tx=['tx','send','--key',str(key),'--to',shared.GENESIS,'--amount','23']
  drafted=cli(tx+['--dry-run']);assert drafted['transaction']['body']['nonce']=='0'
  report['submitted']=cli(tx)
  refusal=cli(tx,False);assert 'pending transaction' in refusal,refusal;report['pending_refusal']=refusal
  mine();account=cli(['rpc','/api/state/account/'+identity['address']]);assert account['nonce']=='1' and account['balance']==str(2*1048576-23),account
  receiver=cli(['rpc','/api/state/account/'+shared.GENESIS]);assert receiver['balance']=='23',receiver
  cookie=node.cookie.read_bytes();node.stop();node.start();assert node.cookie.read_bytes()!=cookie
  report['restarted_status']=cli(['--chain','Nexus','status']);assert report['restarted_status']['height']=='2'
  after=cli(tx+['--dry-run']);assert after['transaction']['body']['nonce']=='1'
  report['after_restart_nonce']=after['transaction']['body']['nonce'];report['account']=account;node.stop()
 assert snapshot()==before,'sources changed during integration run; repeat at stable checkpoint'
 report.update(passed=True,completed=time.time(),inputs_sha256=before)
finally:
 for node in list(processes):node.process.kill();node.process.wait();node.log.close()
 save()
print('CLI signed payment, pending-nonce refusal and restart passed')
